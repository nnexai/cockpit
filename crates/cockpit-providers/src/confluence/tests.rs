use super::{ConfluenceInput, attachment_id_valid, parse_confluence_input};
use url::Url;

#[test]
fn input_recognizes_cloud_and_context_path_dc_without_network() {
    let cloud = Url::parse("https://nnexai.atlassian.net/wiki").unwrap();
    let dc = Url::parse("https://confluence.example.com/confluence").unwrap();
    let page = |id: &str| ConfluenceInput::Page { page_id: id.into() };
    let space = |key: &str| ConfluenceInput::Space {
        space_key: key.into(),
    };
    let display = |title: &str| ConfluenceInput::Display {
        space_key: "ENG".into(),
        title: title.into(),
    };
    for (base, input, expected) in [
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/spaces/SD/pages/123456789/Release+Checklist",
            page("123456789"),
        ),
        (
            &cloud,
            "https://NNEXAI.atlassian.net/wiki/spaces/SD/pages/123456789",
            page("123456789"),
        ),
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/pages/viewpage.action?pageId=42",
            page("42"),
        ),
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/spaces/SD",
            space("SD"),
        ),
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/spaces/~5af4129c/overview",
            space("~5af4129c"),
        ),
        (&cloud, " 123456789 ", page("123456789")),
        (&cloud, "SD", space("SD")),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/Release+Checklist",
            display("Release Checklist"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/Parent/Child%20Page",
            display("Child Page"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/-draft+notes",
            display("-draft notes"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/a%2Bb",
            display("a+b"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG",
            space("ENG"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=524301",
            page("524301"),
        ),
    ] {
        assert_eq!(
            parse_confluence_input(base, input).unwrap(),
            expected,
            "{input}"
        );
    }
    for (base, input, code) in [
        (
            &dc,
            "https://confluence.example.com/confluence/x/DQAI",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=1&pageId=2",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=1a",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/%FF",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/a%0Ab",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/other/display/ENG/X",
            "source_identity_mismatch",
        ),
        (
            &dc,
            "http://confluence.example.com/confluence/display/ENG/X",
            "source_identity_mismatch",
        ),
        (
            &dc,
            "https://confluence.example.com:8443/confluence/display/ENG/X",
            "source_identity_mismatch",
        ),
        (
            &cloud,
            "https://user:pw@nnexai.atlassian.net/wiki/spaces/SD",
            "source_identity_mismatch",
        ),
        (
            &cloud,
            "https://evil.atlassian.net/wiki/spaces/SD/pages/1",
            "source_identity_mismatch",
        ),
        (
            &cloud,
            "123456789012345678901",
            "library_input_unrecognized",
        ),
        (&cloud, "release notes", "library_input_unrecognized"),
        (
            &cloud,
            "ftp://nnexai.atlassian.net/wiki/spaces/SD",
            "library_input_unrecognized",
        ),
    ] {
        assert_eq!(
            parse_confluence_input(base, input).unwrap_err().code,
            code,
            "{input}"
        );
    }
}

#[test]
fn attachment_ids_accept_cloud_att_prefix_and_refuse_non_ids() {
    for id in ["0", "557057", "att557057", "att12345678901234567890"] {
        assert!(attachment_id_valid(id));
    }
    for id in [
        "",
        "att",
        "ATT1",
        "att-1",
        "att123456789012345678901",
        "../1",
        "att1/2",
    ] {
        assert!(!attachment_id_valid(id));
    }
}
