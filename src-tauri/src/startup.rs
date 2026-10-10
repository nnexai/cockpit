use std::sync::Arc;

use cockpit_core::{
    CockpitService,
    config::{BrowserSection, ConfigurationFile, LibrarySyncConfiguration, WindowConfiguration},
    credentials::ProviderCredentials,
    library::{LibraryService, LibrarySyncRuntime},
    projects::ProjectService,
    quota::QuotaService,
    sources::SourceService,
};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_host::{BrowserRuntime, OrchestrationRuntime};
use cockpit_protocol::{projects::ProjectConfiguration, v1::CockpitMode};

pub struct NativeServices {
    pub service: CockpitService,
    pub browser_runtime: Arc<BrowserRuntime>,
    pub orchestration_runtime: Arc<OrchestrationRuntime>,
    pub shutdown_projects: Arc<ProjectService>,
    pub library: Arc<LibraryService>,
    pub library_sync_config: LibrarySyncConfiguration,
    pub window_config: WindowConfiguration,
    pub startup_inspector: Arc<HerdrCliAdapter>,
}

struct CoreServices {
    inspector: Arc<HerdrCliAdapter>,
    startup_inspector: Arc<HerdrCliAdapter>,
    project_config: ProjectConfiguration,
    library_sync_config: LibrarySyncConfiguration,
    window_config: WindowConfiguration,
    quota: Arc<QuotaService>,
    credentials: Arc<ProviderCredentials>,
    sources: Arc<SourceService>,
    project_service: ProjectService,
    browser_config: BrowserSection,
}

struct RuntimeServices {
    native: NativeServices,
    project_config: ProjectConfiguration,
    inspector: Arc<HerdrCliAdapter>,
}

pub fn compose() -> NativeServices {
    compose_views(compose_runtimes(compose_core()))
}

fn compose_core() -> CoreServices {
    let configuration = ConfigurationFile::load(None)
        .unwrap_or_else(|error| panic!("failed to load native window configuration: {error}"));
    let window_config = configuration
        .window
        .resolve()
        .unwrap_or_else(|error| panic!("failed to load native window configuration: {error}"));
    let config =
        HerdrCliConfig::from_options(None, None, None).expect("failed to load Herdr configuration");
    let inspector = Arc::new(HerdrCliAdapter::new(config).with_server_autostart());
    let startup_inspector = Arc::clone(&inspector);
    let project_config = configuration
        .project
        .resolve(None)
        .expect("failed to load project configuration");
    let library_sync_config = configuration
        .library_sync
        .resolve()
        .expect("failed to load Library synchronization configuration");
    let quota_config = configuration
        .quota
        .resolve()
        .expect("failed to load quota configuration");
    let quota = Arc::new(QuotaService::new(
        quota_config,
        std::path::Path::new(&project_config.cache_root),
    ));
    let credentials = Arc::new(ProviderCredentials::new(
        &project_config,
        cockpit_secrets::os_vault(),
        cockpit_providers::credential_kinds,
    ));
    let sources = Arc::new(
        SourceService::new(
            &project_config,
            cockpit_providers::configured_providers(&project_config, credentials.clone())
                .expect("invalid configured source providers"),
        )
        .expect("failed to initialize source providers"),
    );
    let project_service = ProjectService::new(project_config.clone(), inspector.clone())
        .expect("failed to initialize project operations")
        .with_sources(sources.clone());
    CoreServices {
        inspector,
        startup_inspector,
        project_config,
        library_sync_config,
        window_config,
        quota,
        credentials,
        sources,
        project_service,
        browser_config: configuration.browser,
    }
}

fn compose_runtimes(core: CoreServices) -> RuntimeServices {
    let CoreServices {
        inspector,
        startup_inspector,
        project_config,
        library_sync_config,
        window_config,
        quota,
        credentials,
        sources,
        project_service,
        browser_config,
    } = core;
    let browser_config = browser_config
        .resolve()
        .expect("failed to load browser configuration");
    let paste_adapter = inspector.paste_adapter();
    let browser_service = Arc::new(
        cockpit_core::browser::BrowserService::new(
            browser_config,
            std::path::PathBuf::from(&project_config.state_root),
            inspector.clone(),
        )
        .expect("failed to initialize browser service")
        .with_paste_adapter(paste_adapter.clone()),
    );
    let widgets = Arc::new(cockpit_core::widget::WidgetService::new(
        inspector.clone(),
        paste_adapter,
    ));
    let browser_runtime = Arc::new(
        tauri::async_runtime::block_on(BrowserRuntime::start(
            std::path::PathBuf::from(&project_config.state_root),
            browser_service,
            widgets,
        ))
        .expect("failed to initialize browser runtime"),
    );
    let service = CockpitService::new(CockpitMode::Normal, inspector.clone())
        .with_projects(project_service)
        .with_credentials(credentials)
        .with_quota(quota);
    let warm_projects = service
        .projects()
        .expect("project operations configured")
        .clone();
    tauri::async_runtime::spawn(async move {
        warm_projects.prewarm_repositories();
    });
    let shutdown_projects = service
        .projects()
        .expect("project operations configured")
        .clone();
    let library = Arc::new(
        LibraryService::new(project_config.clone(), sources).with_herdr(inspector.clone()),
    );
    let orchestration_runtime = Arc::new(
        OrchestrationRuntime::new(
            &project_config,
            inspector.clone(),
            shutdown_projects.clone(),
            library.clone(),
            None,
            cockpit_host::orchestration_runtime::CliProcessRole::Native,
        )
        .expect("failed to initialize orchestration"),
    );
    tauri::async_runtime::block_on(async {
        orchestration_runtime
            .start_owner(browser_runtime.is_owner().await)
            .await;
    });
    let service = service.with_orchestration(orchestration_runtime.service.clone());
    RuntimeServices {
        native: NativeServices {
            service,
            browser_runtime,
            orchestration_runtime,
            shutdown_projects,
            library,
            library_sync_config,
            window_config,
            startup_inspector,
        },
        project_config,
        inspector,
    }
}

fn compose_views(runtimes: RuntimeServices) -> NativeServices {
    let RuntimeServices {
        mut native,
        project_config,
        inspector,
    } = runtimes;
    let contexts = cockpit_core::context::ContextService::new(
        project_config.clone(),
        inspector.source_adapter(),
        native.shutdown_projects.clone(),
    )
    .with_library(native.library.clone());
    let viewers = Arc::new(cockpit_core::viewer::ViewerService::new(Arc::new(
        contexts.clone(),
    )));
    let contexts = contexts.with_viewers(viewers.clone());
    let service = native.service.with_contexts(contexts).with_viewers(viewers);
    let service = service.with_library((*native.library).clone());
    let service = service.with_notes(
        cockpit_core::notes::NotesService::new(std::path::PathBuf::from(
            &project_config.notes_root,
        ))
        .with_herdr(inspector.clone()),
    );
    let reviews = cockpit_core::review::ReviewService::new(
        project_config.clone(),
        service
            .contexts()
            .expect("context operations configured")
            .clone(),
    )
    .expect("failed to initialize review operations");
    let service = service.with_reviews(reviews);
    let comments = cockpit_core::comments::CommentsService::new(
        project_config,
        service
            .contexts()
            .expect("context operations configured")
            .clone(),
    )
    .unwrap_or_else(|error| panic!("failed to initialize comment operations: {error}"))
    .with_paste_adapter(inspector.paste_adapter())
    .with_reviews(
        service
            .reviews()
            .expect("review operations configured")
            .clone(),
    );
    native.service = service.with_comments(comments);
    native
}

pub fn start_library_sync(
    library: &LibraryService,
    configuration: LibrarySyncConfiguration,
) -> LibrarySyncRuntime {
    // Enter the same Tokio runtime as other native services before spawning.
    // The event callback retains this guard until exit and cancels it before
    // shutting down services that background synchronization can use.
    tauri::async_runtime::block_on(async {
        library
            .start_sync(configuration)
            .expect("failed to initialize Library synchronization")
    })
}
