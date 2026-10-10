use std::{sync::Arc, time::Duration};

use cockpit_core::projects::ProjectService;

use crate::{BrowserRuntime, OrchestrationRuntime};

#[derive(Clone)]
pub struct HostShutdown {
    orchestration: Option<Arc<OrchestrationRuntime>>,
    projects: Option<Arc<ProjectService>>,
    browser: Option<Arc<BrowserRuntime>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownPolicy {
    Gateway,
    Native,
}

impl HostShutdown {
    pub fn new(
        orchestration: Option<Arc<OrchestrationRuntime>>,
        projects: Option<Arc<ProjectService>>,
        browser: Option<Arc<BrowserRuntime>>,
    ) -> Self {
        Self {
            orchestration,
            projects,
            browser,
        }
    }

    pub async fn run(&self, policy: ShutdownPolicy) {
        self.orchestration().await;
        match policy {
            ShutdownPolicy::Gateway => {
                if let Some(projects) = &self.projects {
                    projects.shutdown().await;
                }
                if let Some(browser) = &self.browser {
                    let _ = browser.shutdown().await;
                }
            }
            ShutdownPolicy::Native => {
                // Stop the owned helper before waiting on workspace
                // operations that may be waiting on an external Herdr
                // response. Neither shutdown path may keep app exit open.
                if let Some(browser) = &self.browser {
                    let _ = browser.shutdown().await;
                }
                if let Some(projects) = &self.projects {
                    let _ = tokio::time::timeout(Duration::from_secs(5), projects.shutdown()).await;
                }
            }
        }
    }

    /// Gateway calls this again after serving ends, including on a serve error.
    pub async fn orchestration(&self) {
        if let Some(orchestration) = &self.orchestration {
            orchestration.shutdown().await;
        }
    }
}
