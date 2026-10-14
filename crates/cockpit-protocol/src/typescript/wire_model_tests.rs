use std::sync::LazyLock;

use serde_json::json;

use super::{Section, WireField, WireShape, WireTy, WireType, wire_model};

fn model() -> &'static [WireType] {
    static MODEL: LazyLock<Vec<WireType>> =
        LazyLock::new(|| wire_model(super::super::SECTIONS).expect("real DTO source model"));
    &MODEL
}

fn dto(name: &str) -> &'static WireType {
    model()
        .iter()
        .find(|ty| ty.name == name)
        .expect("listed DTO")
}

fn fields(name: &str) -> &'static [WireField] {
    let WireShape::Struct(fields) = &dto(name).shape else {
        panic!("expected struct {name}");
    };
    fields
}

fn field(name: &str, key: &str) -> &'static WireField {
    fields(name)
        .iter()
        .find(|field| field.name == key)
        .expect("source field")
}

fn tagged(name: &str) -> (&'static str, &'static [(String, Vec<WireField>)]) {
    let WireShape::Tagged { tag, variants } = &dto(name).shape else {
        panic!("expected tagged {name}");
    };
    (tag, variants)
}

fn fixture(source: &'static str, names: &'static [&'static str]) -> Result<Vec<WireType>, String> {
    let section = Section {
        module: "fixture",
        source,
        names,
        decls: |_, _| {},
    };
    wire_model(&[&section])
}

#[test]
fn source_defaults_and_serialization_presence_are_preserved() {
    let quota = field("QuotaStatusRequest", "agents_working");
    assert_eq!(quota.fill, Some(json!(false)));
    assert!(!quota.ts_optional);
    assert!(dto("QuotaStatusRequest").deny_unknown_fields);
    for (key, default) in [
        ("reference_depth", json!(0)),
        ("follow", json!(false)),
        ("follow_mode", json!(null)),
        ("download_attachments", json!(false)),
        ("refresh_existing", json!(false)),
    ] {
        assert_eq!(field("LibraryAddRequest", key).fill, Some(default));
    }
    assert_eq!(
        field("TerminalOpenRequest", "target_kind").fill,
        Some(json!("pane"))
    );
    assert!(field("TerminalOpenRequest", "target_kind").ts_optional);
    let tab = field("BrowserPageEvidence", "tab_id");
    assert!(tab.ts_optional && tab.rust_omits && !tab.ts_nonnull);
    assert_eq!(tab.fill, Some(json!(null)));
    assert!(matches!(tab.ty, WireTy::Option(_)));
    assert_eq!(
        field("ProjectConfiguration", "orchestration").fill,
        Some(json!({
            "harness_kind": "omp", "command": "omp", "extra_args": [],
            "resume_args": null, "initial_prompt_args": null, "hook_args": null,
            "max_parallel": 4, "process_scope": "auto", "inbox_prompt_hints": false, "routes": []
        }))
    );
    assert_eq!(field("OrchestrationConfiguration", "harness_kind").fill, Some(json!("omp")));
    assert_eq!(field("OrchestrationConfiguration", "command").fill, Some(json!("omp")));
}

#[test]
fn launch_configuration_defaults_follow_literal_source_and_reject_computation() {
    let source = r#"
        fn default_harness() -> String { "custom".into() }
        fn default_max_parallel() -> u32 { 7 }
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "snake_case")]
        enum ScopeMode { #[default] #[serde(rename = "automatic")] Auto, Off }
        #[derive(Deserialize)]
        struct OrchestrationConfiguration {
            #[serde(default = "default_harness")] harness_kind: String,
            #[serde(default = "default_harness")] command: String,
            extra_args: Vec<String>, resume_args: Option<Vec<String>>,
            initial_prompt_args: Option<Vec<String>>, hook_args: Option<Vec<String>>,
            #[serde(default = "default_max_parallel")] max_parallel: u32,
            process_scope: ScopeMode, inbox_prompt_hints: bool, routes: Vec<String>,
        }
        impl Default for OrchestrationConfiguration {
            fn default() -> Self {
                Self {
                    harness_kind: default_harness(), command: default_harness(),
                    extra_args: Vec::new(), resume_args: None, initial_prompt_args: None, hook_args: None,
                    max_parallel: default_max_parallel(), process_scope: ScopeMode::Auto,
                    inbox_prompt_hints: false, routes: Vec::new()
                }
            }
        }
    "#;
    let file = syn::parse_file(source).unwrap();
    let configuration = super::source::orchestration_defaults(&file).unwrap();
    assert_eq!(configuration, json!({
        "harness_kind": "custom", "command": "custom", "extra_args": [],
        "resume_args": null, "initial_prompt_args": null, "hook_args": null,
        "max_parallel": 7, "process_scope": "automatic", "inbox_prompt_hints": false, "routes": []
    }));
    for changed in [
        source.replace("\"custom\".into()", "std::env::var(\"HARNESS\").unwrap()"),
        source.replace("Vec::new()", "compute_args()"),
        source.replace("resume_args: None,", ""),
        source.replace("-> u32 { 7 }", "-> u32 { 1 + 6 }"),
        source.replace("\"custom\".into()", "7"),
        source.replace("-> u32 { 7 }", "-> u32 { 4294967296 }"),
        source.replace("-> u32 { 7 }", "-> u32 { default_max_parallel() }"),
        source.replace("-> u32 { 7 }", "-> u32 { default_harness() }")
            .replace("\"custom\".into()", "default_max_parallel()"),
        source.replace("process_scope: ScopeMode::Auto,", "process_scope: ScopeMode::Missing,"),
        source.replace("max_parallel: default_max_parallel(),", "max_parallel: 7,"),
        source.replace("fn default_max_parallel()", "fn default_max_parallel(limit: u32)"),
        source.replace("-> u32 { 7 }", "-> u32 { let limit = 7; limit }"),
        source.replace(" Auto, Off", " Auto { limit: u32 }, Off"),
    ] {
        let file = syn::parse_file(&changed).unwrap();
        assert!(super::source::orchestration_defaults(&file).is_err());
    }
}

#[test]
fn orchestration_config_and_optional_scope_defaults_match_rust_decoding() {
    let decoded: crate::projects::OrchestrationConfiguration =
        serde_json::from_value(json!({})).expect("omitted config fields decode");
    let defaults = serde_json::to_value(decoded).expect("serialize current defaults");
    assert_eq!(field("ProjectConfiguration", "orchestration").fill, Some(defaults.clone()));
    for source_field in fields("OrchestrationConfiguration") {
        assert_eq!(source_field.fill.as_ref(), defaults.get(&source_field.name),
            "config default {}", source_field.name);
    }
    assert_eq!(dto("ScopeMode").shape, WireShape::Literals(vec!["auto".into(), "off".into()]));
    for (owner, key) in [
        ("TaskOwner", "conversation"), ("TaskOwnership", "schedule"),
        ("ExecutionAttachment", "process_scope"), ("DispatchState", "scope_unit"),
        ("DispatchState", "latch"), ("DispatchState", "latch_state"),
        ("DispatchState", "source_location"), ("DispatchState", "move_operation_id"),
        ("DispatchState", "staging_workspace_id"),
        ("DispatchState", "pre_move_scope"),
        ("RunLocation", "peer_process"), ("RetirementIdentity", "herdr_peer"),
        ("RunRetirement", "result_message_id"), ("RunRetirement", "task_revision"),
        ("RunRetirement", "replacement"),
    ] {
        let source_field = field(owner, key);
        assert!(matches!(source_field.ty, WireTy::Option(_)));
        assert_eq!(source_field.fill, Some(json!(null)), "{owner}.{key}");
    }
    let (_, results) = tagged("OrchestrationActionResult");
    let registered = &results.iter().find(|(name, _)| name == "run_registered").unwrap().1;
    assert_eq!(registered.iter().find(|field| field.name == "conversation").unwrap().fill, Some(json!(null)));
}

#[test]
fn attempt_directory_identity_is_distinct_and_requires_only_its_creation_pin() {
    let identity = fields("OwnedAttemptDirectoryIdentity");
    assert_eq!(identity.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["creation_operation_id", "root", "device", "inode"]);
    assert!(identity.iter().all(|field| field.fill.is_none() && !field.ts_optional));
    assert!(dto("OwnedAttemptDirectoryIdentity").deny_unknown_fields);
    assert!(fields("OwnedFilesystemIdentity").iter().any(|field| field.name == "manifest_path"));
    let (_, resources) = tagged("OwnedResourceIdentity");
    let attempt = &resources.iter().find(|(name, _)| name == "attempt_directory").unwrap().1;
    assert_eq!(attempt.len(), 1);
    assert_eq!(attempt[0].ty, WireTy::Named("OwnedAttemptDirectoryIdentity".into()));
}

#[test]
fn session_ownership_actions_and_results_are_source_grounded() {
    assert_eq!(
        fields("ConversationIdentity").iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["harness", "session_id"]
    );
    assert_eq!(field("TaskOwnership", "owner").ty, WireTy::Option(Box::new(WireTy::Named("TaskOwner".into()))));
    assert_eq!(field("TaskOwner", "conversation").ty, WireTy::Option(Box::new(WireTy::Named("ConversationIdentity".into()))));
    assert_eq!(field("ExecutionAttachment", "conversation").ty, WireTy::Option(Box::new(WireTy::Named("ConversationIdentity".into()))));
    assert_eq!(field("RunObservation", "actual_agent").ty, WireTy::Bool);
    assert_eq!(field("Run", "launch_config").ty, WireTy::Named("OrchestrationConfiguration".into()));
    let (tag, actions) = tagged("OrchestrationAction");
    assert_eq!(tag, "action");
    let register = &actions.iter().find(|(name, _)| name == "run_register").expect("registration action").1;
    assert_eq!(register.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(), ["operation_id"]);
    let unassign = &actions.iter().find(|(name, _)| name == "task_unassign").expect("unassign action").1;
    assert_eq!(unassign.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["operation_id", "root_id", "task_id", "expected_assignment_generation"]);
    assert!(actions.iter().all(|(name, _)| !matches!(name.as_str(),
        "job_child_register" | "retirement_native_receipt" | "inbox_woken" |
        "subagent_update" | "subagent_control" | "subagent_control_complete")));
    let (tag, results) = tagged("OrchestrationActionResult");
    assert_eq!(tag, "result");
    let registered = &results.iter().find(|(name, _)| name == "run_registered").expect("registration result").1;
    assert_eq!(registered.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["run_id", "conversation", "assignment_generation", "duplicate"]);
    let unassigned = &results.iter().find(|(name, _)| name == "task_unassigned").expect("unassign result").1;
    assert_eq!(unassigned.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["task_id", "assignment_generation", "previous_run_id", "duplicate"]);
    assert!(model().iter().all(|ty| !matches!(ty.name.as_str(),
        "Subagent" | "SubagentStatus" | "ControlStage" | "SubagentControlState" | "SubagentOp" | "AgentKind")));
}

#[test]
fn orchestration_launch_configuration_uses_cross_domain_descriptor_import() {
    let files = super::super::validators::render(model()).expect("render source descriptors");
    let (_, run_descriptor) = files.iter()
        .find(|(_, content)| content.contains("export const wireRun:"))
        .expect("run descriptor");
    assert!(run_descriptor.lines().any(|line|
        line.starts_with("import { ")
            && line.contains("wireOrchestrationConfiguration")
            && line.contains("from \"./projects")));
    assert!(run_descriptor.contains("named(() => wireOrchestrationConfiguration)"));
}

#[test]
fn tagged_alias_and_serde_field_names_are_source_grounded() {
    let (tag, variants) = tagged("TerminalCommand");
    assert_eq!(tag, "type");
    assert_eq!(
        variants
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "terminal.input",
            "terminal.resize",
            "terminal.scroll",
            "terminal.mouse",
            "terminal.release"
        ]
    );
    assert_eq!(variants[0].1[0].fill, Some(json!(null)));
    assert_eq!(tagged("HerdrCompatibility").0, "status");
    let (tag, variants) = tagged("NotesTodoSelector");
    assert_eq!(tag, "by");
    let (_, fields) = variants
        .iter()
        .find(|(tag, _)| tag == "ref")
        .expect("ref variant");
    assert_eq!(fields[0].name, "ref");
    assert_eq!(fields[0].ty, WireTy::Str);
}

#[test]
fn retirement_state_uses_invocation_order_and_types() {
    let (tag, variants) = tagged("RetirementState");
    assert_eq!(tag, "state");
    assert_eq!(
        variants
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "waiting",
            "close_intent",
            "retired",
            "retained",
            "unknown"
        ]
    );
    assert_eq!(
        variants[0].1[0].ty,
        WireTy::List(Box::new(WireTy::Named("RetirementBlocker".into())))
    );
    assert_eq!(variants[2].1[1].ty, WireTy::Named("TerminalOutcome".into()));
    assert_eq!(variants[3].1[1].ty, WireTy::Named("RetainReason".into()));
    assert_eq!(variants[4].1[1].ty, WireTy::Named("RetirementPhase".into()));
    assert_eq!(variants[3].1[2].name, "stopped");
    assert_eq!(variants[3].1[2].ty, WireTy::Bool);
    for (name, child) in [
        ("conversation", "ConversationIdentity"),
        ("process", "NativeProcessIdentity"),
        ("shell", "NativeShellIdentity"),
    ] {
        assert_eq!(field("RetirementIdentity", name).ty, WireTy::Option(Box::new(WireTy::Named(child.into()))));
    }
    assert!(fields("RunRetirement").iter().all(|field| field.name != "quiescent_stop_at"));
}

#[test]
fn integer_widths_are_not_inferred_from_typescript_numbers() {
    let model = fixture(
        "#[derive(Deserialize)] struct Bounds { a:u8,b:u16,c:u32,d:u64,e:usize,f:i64 }",
        &["Bounds"],
    )
    .expect("supported integers");
    let WireShape::Struct(fields) = &model[0].shape else {
        panic!("struct");
    };
    let safe = 9_007_199_254_740_991;
    assert_eq!(
        fields
            .iter()
            .map(|field| field.ty.clone())
            .collect::<Vec<_>>(),
        [
            WireTy::Int { min: 0, max: 255 },
            WireTy::Int {
                min: 0,
                max: 65_535
            },
            WireTy::Int {
                min: 0,
                max: 4_294_967_295
            },
            WireTy::Int { min: 0, max: safe },
            WireTy::Int { min: 0, max: safe },
            WireTy::Int {
                min: -safe,
                max: safe
            },
        ]
    );
}

#[test]
fn explicit_optional_and_type_overrides_follow_ts_rs_nullability() {
    let model = fixture(
        r#"#[derive(Deserialize)] struct Optional {
        #[ts(optional)] a:Option<String>,
        #[ts(optional = nullable)] b:Option<String>,
        #[serde(default,skip_serializing_if="Option::is_none")] c:Option<String>,
        #[ts(optional,type="number | null")] d:Option<u64>,
    }"#,
        &["Optional"],
    )
    .expect("source option rules");
    let WireShape::Struct(fields) = &model[0].shape else {
        panic!("struct");
    };
    assert!(fields.iter().all(|field| field.ts_optional));
    assert_eq!(
        fields
            .iter()
            .map(|field| field.ts_nonnull)
            .collect::<Vec<_>>(),
        [true, false, false, false]
    );
}

#[test]
fn macro_shape_and_missing_listed_type_fail_with_named_errors() {
    let missing = fixture("", &["MissingDto"]).unwrap_err();
    assert!(missing.contains("MissingDto"));
    let mismatch = fixture(
        r#"
        macro_rules! retirement_states {
            ($variant:ident) => {
                #[derive(Deserialize)] #[serde(tag="state")]
                pub enum RetirementState { $variant {} }
            };
        }
        retirement_states! { Waiting {} }
    "#,
        &["RetirementState"],
    )
    .unwrap_err();
    assert!(mismatch.contains("retirement_states"));
    let changed_body = fixture(
        r#"
        macro_rules! retirement_states {
            ($($variant:ident { $($field:ident: $field_type:ty),* $(,)? }),* $(,)?) => {
                #[derive(Deserialize)] #[serde(tag="state")]
                pub enum RetirementState { $($variant),* }
            };
        }
        retirement_states! { Waiting {} }
    "#,
        &["RetirementState"],
    )
    .unwrap_err();
    assert!(changed_body.contains("retirement_states"));
}

#[test]
fn manual_defaults_are_not_guessed_and_custom_defaults_fail_closed() {
    let model = fixture(
        r#"
        #[derive(Deserialize)] #[serde(default)] struct Manual { x:bool }
        impl Default for Manual { fn default()->Self { Self { x:true } } }
    "#,
        &["Manual"],
    )
    .expect("manual default remains unknown");
    let WireShape::Struct(fields) = &model[0].shape else {
        panic!("struct");
    };
    assert_eq!(fields[0].fill, None);
    let error = fixture(
        r#"#[derive(Deserialize)] struct Custom {
        #[serde(default="custom_fill")] x:bool
    }"#,
        &["Custom"],
    )
    .unwrap_err();
    assert!(error.contains("Custom.x") && error.contains("custom_fill"));
    let unknown = fixture(
        "#[derive(Deserialize)] struct Unknown { x:Unsupported }",
        &["Unknown"],
    )
    .unwrap_err();
    assert!(unknown.contains("Unknown.x") && unknown.contains("Unsupported"));
}

#[test]
fn serde_ts_rename_disagreement_is_named() {
    let error = fixture(
        r#"#[derive(Deserialize)] struct Renamed {
        #[serde(rename="wire")] #[ts(rename="other")] source:String
    }"#,
        &["Renamed"],
    )
    .unwrap_err();
    assert!(error.contains("Renamed.wire") && error.contains("ts rename"));
}

#[test]
fn alias_uses_public_ts_presence_not_private_wire_attributes() {
    let model = fixture(
        r#"
        #[derive(TS)] #[ts(tag="type")] enum TerminalCommand {
            #[ts(rename="terminal.input")] Input { text:Option<String> },
        }
        #[derive(Deserialize)] #[serde(tag="type")] enum TerminalCommandWire {
            #[serde(rename="terminal.input")] Input {
                #[serde(default,skip_serializing_if="Option::is_none")] text:Option<String>,
            },
        }
        impl<'de> Deserialize<'de> for TerminalCommand {
            fn deserialize<D>(deserializer:D)->Result<Self,D::Error>
            where D:serde::Deserializer<'de> {
                let wire = TerminalCommandWire::deserialize(deserializer)?;
                Self::try_from(wire).map_err(serde::de::Error::custom)
            }
        }
    "#,
        &["TerminalCommand"],
    )
    .expect("validated manual delegate");
    let WireShape::Tagged { variants, .. } = &model[0].shape else {
        panic!("tagged alias");
    };
    let field = &variants[0].1[0];
    assert!(!field.ts_optional && !field.ts_nonnull);
    assert!(field.rust_omits);
    assert_eq!(field.fill, Some(json!(null)));
}
