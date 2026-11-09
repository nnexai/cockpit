use super::archive::*;
use super::store::OrchestrationStore;
use cockpit_protocol::orchestration::{ActorRef, DeliveryStage, Message, MessageKind};

fn message(text: &str) -> Message {
    Message { message_id: uuid::Uuid::new_v4().to_string(), to_run_id: "recipient".into(), seq: 1,
        from: ActorRef::Operator, kind: MessageKind::Instruction, text: text.into(), images: vec![],
        in_reply_to: None, report: None, thread_id: None, handoff: None, stale: false,
        escalated_from: None, stage: DeliveryStage::Acked, created_at: "time".into(), acked_at: Some("time".into()) }
}

#[test]
fn positions_survive_multiple_segments_and_appends() {
    let path = std::env::temp_dir().join(format!("cockpit-archive-test-{}", uuid::Uuid::new_v4()));
    let store = OrchestrationStore::open(&path).unwrap();
    let lock = store.lock().unwrap();
    let mut state = lock.read().unwrap();
    state.messages = vec![message("a"), message("b"), message("c"), message("d"), message("e")];
    state.messages[2].message_id = state.messages[1].message_id.clone();
    state.messages[2].from = ActorRef::Run { run_id: "other-sender".into() };
    let first = test_segment(&lock, vec![(1, state.messages[1].clone()), (3, state.messages[3].clone())]);
    state.archives.push(first);
    lock.save(&mut state).unwrap();
    let mut state = lock.read().unwrap();
    state.messages.push(message("f"));
    let second = test_segment(&lock, vec![(2, state.messages[2].clone()), (4, state.messages[4].clone())]);
    state.archives.push(second);
    lock.save(&mut state).unwrap();
    let mut state = lock.read().unwrap();
    assert_eq!(state.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d", "e", "f"]);
    state.archives.remove(0);
    lock.save(&mut state).unwrap();
    let mut state = lock.read().unwrap();
    assert_eq!(state.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d", "e", "f"]);
    let again = test_segment(&lock, vec![(1, state.messages[1].clone()), (3, state.messages[3].clone())]);
    state.archives.push(again);
    lock.save(&mut state).unwrap();
    let mut state = lock.read().unwrap();
    assert_eq!(state.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d", "e", "f"]);
    state.messages[1].text = "changed".into();
    assert_eq!(lock.save(&mut state).unwrap_err().code, "archive_conflict");
    drop(lock);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn archived_cache_reuses_hydration_but_revalidates_segment_bytes() {
    let path = std::env::temp_dir().join(format!("cockpit-archive-cache-{}", uuid::Uuid::new_v4()));
    let store = OrchestrationStore::open(&path).unwrap();
    let lock = store.lock().unwrap();
    let mut state = lock.read().unwrap();
    state.messages.push(message("archived"));
    let reference = test_segment(&lock, vec![(0, state.messages[0].clone())]);
    let id = serde_json::to_value(&reference).unwrap()["id"].as_str().unwrap().to_owned();
    state.archives.push(reference);
    let revision = lock.save(&mut state).unwrap();
    let first = lock.read_shared().unwrap();
    let second = lock.read_shared().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(lock.read().unwrap().messages.len(), 1);
    let segment = path.join("orchestration").join(format!("archive-{id}.json"));
    let original = std::fs::read(&segment).unwrap();
    let mut altered = original.clone();
    altered[0] ^= 1;
    std::fs::write(&segment, altered).unwrap();
    assert_eq!(lock.read_shared().unwrap_err().code, "archive_corrupt");
    assert_eq!(lock.read().unwrap_err().code, "archive_corrupt");
    assert_eq!(lock.inbox_counts("recipient", 0).err().unwrap().code, "archive_corrupt");
    assert_eq!(lock.wait_revision().unwrap(), revision);
    std::fs::write(&segment, original).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &lock.read_shared().unwrap()));
    drop(lock);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}
