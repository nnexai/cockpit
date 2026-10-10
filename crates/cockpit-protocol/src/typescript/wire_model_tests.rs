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
        Some(json!({ "omp_extension": null, "model": null, "extra_args": [], "routes": [] }))
    );
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
            "native_stop_offered",
            "native_stop_deferred",
            "native_stop_requested",
            "native_stopped",
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
    assert_eq!(
        variants[4].1[1].ty,
        WireTy::Named("NativeStopEvidence".into())
    );
    assert_eq!(variants[6].1[1].ty, WireTy::Named("TerminalOutcome".into()));
    assert_eq!(variants[7].1[2].ty, WireTy::Bool);
    assert_eq!(variants[8].1[1].ty, WireTy::Named("RetirementPhase".into()));
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
