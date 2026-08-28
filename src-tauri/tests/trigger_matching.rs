use callback_lib::platform::focus::{BrowserContext, FocusTarget, OsFocus, combine_focus};
use callback_lib::triggers::{
    Candidate, LinkInput, TriggerKind, auto_link, matching_priority, select_one,
};

#[test]
fn context_trigger_outranks_app_fallback() {
    let triggers = auto_link(LinkInput {
        source_app: "slack",
        source_ctx: Some("D0123"),
        text: "I will send the invoice",
        keyword_app_map: &[],
    });
    assert_eq!(triggers[0].kind, TriggerKind::AppCtxFocus);
    assert_eq!(triggers[0].priority, 100);
    assert_eq!(triggers[1].kind, TriggerKind::AppFocus);
}

#[test]
fn keyword_map_adds_editor_fallback() {
    let triggers = auto_link(LinkInput {
        source_app: "slack",
        source_ctx: None,
        text: "I'll push the fix",
        keyword_app_map: &[("push".into(), "Code.exe".into())],
    });
    assert!(
        triggers
            .iter()
            .any(|trigger| trigger.match_value.eq_ignore_ascii_case("Code.exe"))
    );
}

#[test]
fn chrome_without_matching_tab_does_not_match_slack_context() {
    let os = OsFocus {
        exe_name: "chrome.exe".into(),
    };
    let target = combine_focus(Some(&os), None);
    let triggers = auto_link(LinkInput {
        source_app: "slack",
        source_ctx: Some("D0123"),
        text: "I will send it",
        keyword_app_map: &[],
    });
    assert!(matching_priority(&target, &triggers[0]).is_none());
}

#[test]
fn matching_visible_slack_tab_hits_context_trigger() {
    let os = OsFocus {
        exe_name: "chrome.exe".into(),
    };
    let browser = BrowserContext {
        source_app: "slack".into(),
        source_ctx: Some("D0123".into()),
        visible: true,
        active: true,
    };
    let target = combine_focus(Some(&os), Some(&browser));
    let triggers = auto_link(LinkInput {
        source_app: "slack",
        source_ctx: Some("D0123"),
        text: "I will send it",
        keyword_app_map: &[],
    });
    assert_eq!(matching_priority(&target, &triggers[0]), Some(100));
}

#[test]
fn stale_heartbeat_does_not_match() {
    let os = OsFocus {
        exe_name: "chrome.exe".into(),
    };
    let browser = BrowserContext {
        source_app: "slack".into(),
        source_ctx: Some("D0123".into()),
        visible: false,
        active: true,
    };
    let target = combine_focus(Some(&os), Some(&browser));
    assert_eq!(target.context, None);
}

#[test]
fn competing_promises_select_nearest_deadline() {
    let mut candidates = [
        Candidate {
            promise_id: 1,
            priority: 10,
            deadline_ts: Some(50),
            confidence: 0.9,
            created_at: 1,
        },
        Candidate {
            promise_id: 2,
            priority: 100,
            deadline_ts: Some(10),
            confidence: 0.2,
            created_at: 2,
        },
    ];
    let winner = select_one(&mut candidates).expect("winner");
    assert_eq!(winner.promise_id, 2);
}

#[test]
fn unused_focus_target_type_stays_public() {
    let _ = FocusTarget {
        app_id: "chrome.exe".into(),
        context: None,
    };
}
