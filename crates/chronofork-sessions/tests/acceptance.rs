#![allow(clippy::expect_used)]

//! End-to-end acceptance tests for the generic session layer.
//!
//! These tests exercise the public API, not backend internals. They are the
//! executable form of the bootstrap's core invariants:
//!
//! - forks start at the checkpoint state;
//! - branches diverge under different inputs and do not mutate each other;
//! - restore plus replay reproduces the original digest;
//! - serialized checkpoints can seed a new manager;
//! - lifecycle guards reject invalid transitions.

use std::sync::Arc;

use chronofork_core::InputFrame;
use chronofork_sessions::{SessionError, SessionManager, Snapshot};
use chronofork_test_backend::SyntheticBackendFactory;

fn manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn step_many(
    manager: &mut SessionManager,
    session: chronofork_core::SessionId,
    input: u32,
    count: usize,
) {
    for _ in 0..count {
        manager
            .step(session, InputFrame::new(input))
            .expect("session step succeeds");
    }
}

#[test]
fn three_way_fork_is_isolated_and_diverges() {
    let mut manager = manager();
    let source = manager.create_session().expect("source session");
    step_many(&mut manager, source, 0, 12);
    let checkpoint = manager.snapshot(source).expect("checkpoint");
    let checkpoint_view = manager.inspect(source).expect("checkpoint inspection");

    let branch_a = manager
        .fork_from_snapshot(source, checkpoint)
        .expect("fork A");
    let branch_b = manager
        .fork_from_snapshot(source, checkpoint)
        .expect("fork B");
    let branch_c = manager
        .fork_from_snapshot(source, checkpoint)
        .expect("fork C");

    for branch in [branch_a, branch_b, branch_c] {
        let view = manager.inspect(branch).expect("fork inspection");
        assert_eq!(view.machine.tick, checkpoint_view.machine.tick);
        assert_eq!(view.state_hash, checkpoint_view.state_hash);
        assert_eq!(view.lineage.parent_session(), Some(source));
        assert_eq!(view.lineage.origin_snapshot(), Some(checkpoint));
        assert_eq!(view.lineage.depth(), 1);
    }

    let inputs = [1_u32, 0x7f, 0xff];
    for (branch, input) in [branch_a, branch_b, branch_c].into_iter().zip(inputs) {
        step_many(&mut manager, branch, input, 6);
    }

    let hash_a = manager.state_hash(branch_a).expect("A inspection digest");
    let hash_b = manager.state_hash(branch_b).expect("B inspection digest");
    let hash_c = manager.state_hash(branch_c).expect("C inspection digest");
    assert_ne!(hash_a, hash_b);
    assert_ne!(hash_b, hash_c);
    assert_ne!(hash_a, hash_c);

    let source_after = manager.inspect(source).expect("source inspection");
    assert_eq!(source_after.machine.tick, checkpoint_view.machine.tick);
    assert_eq!(source_after.state_hash, checkpoint_view.state_hash);

    let memory_a_before = manager
        .read_memory(branch_a, 0x20, 1)
        .expect("read A memory");
    manager
        .write_memory(branch_c, 0x20, &[0xaa])
        .expect("write C memory");
    let memory_a_after = manager
        .read_memory(branch_a, 0x20, 1)
        .expect("read A memory again");
    let memory_c_after = manager
        .read_memory(branch_c, 0x20, 1)
        .expect("read C memory");
    assert_eq!(memory_a_before, memory_a_after);
    assert_eq!(memory_c_after, vec![0xaa]);
}

#[test]
fn restore_and_replay_reproduce_the_branch_state() {
    let mut manager = manager();
    let source = manager.create_session().expect("source session");
    step_many(&mut manager, source, 0, 12);
    let checkpoint = manager.snapshot(source).expect("checkpoint");
    let branch = manager
        .fork_from_snapshot(source, checkpoint)
        .expect("fork branch");

    step_many(&mut manager, branch, 7, 9);
    let original = manager.inspect(branch).expect("branch inspection");
    let checkpoint_hash = manager
        .state_hash(source)
        .expect("source inspection digest");

    manager.restore(branch, checkpoint).expect("restore branch");
    let restored = manager.inspect(branch).expect("restored inspection");
    assert_eq!(restored.machine.tick.raw(), 12);
    assert_eq!(restored.state_hash, checkpoint_hash);

    step_many(&mut manager, branch, 7, 9);
    let replayed = manager.inspect(branch).expect("replayed inspection");
    assert_eq!(replayed.machine.tick, original.machine.tick);
    assert_eq!(replayed.state_hash, original.state_hash);
}

#[test]
fn serialized_checkpoint_can_seed_a_new_manager() {
    let mut first = manager();
    let source = first.create_session().expect("source session");
    step_many(&mut first, source, 3, 15);
    let checkpoint = first.snapshot(source).expect("checkpoint");
    let expected = first.inspect(source).expect("inspection");

    let bytes = first
        .snapshot_bytes(source, checkpoint)
        .expect("serialize checkpoint");
    let decoded = Snapshot::from_bytes(&bytes).expect("decode checkpoint");
    assert_eq!(decoded.tick(), expected.machine.tick);
    assert_eq!(decoded.state_hash(), expected.state_hash);

    let mut second = manager();
    let imported = second
        .create_session_from_snapshot_bytes(&bytes)
        .expect("import checkpoint");
    let imported_view = second.inspect(imported).expect("imported inspection");
    assert_eq!(imported_view.machine.tick, expected.machine.tick);
    assert_eq!(imported_view.state_hash, expected.state_hash);
    assert_eq!(imported_view.lineage.origin_snapshot(), Some(checkpoint));

    step_many(&mut second, imported, 1, 3);
    let advanced = second.inspect(imported).expect("advanced inspection");
    assert!(advanced.machine.tick > imported_view.machine.tick);
}

#[test]
fn fork_current_does_not_mutate_the_source() {
    let mut manager = manager();
    let source = manager.create_session().expect("source session");
    step_many(&mut manager, source, 2, 10);
    let before = manager.inspect(source).expect("source inspection");
    let fork = manager.fork_current(source).expect("fork current");
    let fork_view = manager.inspect(fork).expect("fork inspection");
    assert_eq!(fork_view.state_hash, before.state_hash);
    assert_eq!(fork_view.lineage.parent_session(), Some(source));

    step_many(&mut manager, fork, 9, 4);
    let source_after = manager
        .inspect(source)
        .expect("source inspection after fork steps");
    assert_eq!(source_after.state_hash, before.state_hash);
}

#[test]
fn lifecycle_guards_paused_and_closed_sessions() {
    let mut manager = manager();
    let session = manager.create_session().expect("session");

    manager.pause(session).expect("pause");
    let error = manager
        .step(session, InputFrame::NEUTRAL)
        .expect_err("paused step must fail");
    assert!(matches!(error, SessionError::NotRunning { .. }));

    manager.resume(session).expect("resume");
    manager
        .step(session, InputFrame::NEUTRAL)
        .expect("step after resume");

    manager.reset(session).expect("reset");
    let reset = manager.inspect(session).expect("reset inspection");
    assert_eq!(reset.machine.tick.raw(), 0);

    manager.close(session).expect("close");
    let error = manager
        .inspect(session)
        .expect_err("closed inspect must fail");
    assert!(matches!(error, SessionError::Closed(_)));
}
