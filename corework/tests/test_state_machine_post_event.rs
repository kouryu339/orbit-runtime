use corework::error::Result;
use corework::statemachine::{FnState, SimpleTransition, StateMachine};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_post_event_is_deferred_until_next_tick() -> Result<()> {
    let thinking_started = Arc::new(Notify::new());
    let release_thinking = Arc::new(Notify::new());
    let suspended_entered = Arc::new(AtomicBool::new(false));

    let thinking = {
        let thinking_started = Arc::clone(&thinking_started);
        let release_thinking = Arc::clone(&release_thinking);
        FnState::new("thinking")
            .with_on_enter(move |_ctx| {
                let thinking_started = Arc::clone(&thinking_started);
                let release_thinking = Arc::clone(&release_thinking);
                Box::pin(async move {
                    thinking_started.notify_one();
                    release_thinking.notified().await;
                    Ok(())
                })
            })
            .add_transition(
                "PAUSE",
                Box::new(SimpleTransition::new("PAUSE", "suspended")),
            )
    };

    let suspended = {
        let suspended_entered = Arc::clone(&suspended_entered);
        FnState::new("suspended").with_on_enter(move |_ctx| {
            let suspended_entered = Arc::clone(&suspended_entered);
            Box::pin(async move {
                suspended_entered.store(true, Ordering::SeqCst);
                Ok(())
            })
        })
    };

    let sm = Arc::new(
        StateMachine::builder("post_event_deferred")
            .add_state(Box::new(thinking))
            .add_state(Box::new(suspended))
            .initial_state("thinking")
            .build()
            .await?,
    );

    let sm_for_start = Arc::clone(&sm);
    let start_handle = tokio::spawn(async move { sm_for_start.start().await });

    thinking_started.notified().await;
    sm.post_event("PAUSE");
    sleep(Duration::from_millis(50)).await;

    assert_eq!(sm.current_state(), "thinking");
    assert!(!suspended_entered.load(Ordering::SeqCst));

    release_thinking.notify_one();
    let start_result = start_handle
        .await
        .expect("state machine start task panicked");
    start_result?;

    assert_eq!(sm.current_state(), "thinking");
    assert!(!suspended_entered.load(Ordering::SeqCst));

    sm.tick().await?;

    assert_eq!(sm.current_state(), "suspended");
    assert!(suspended_entered.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn test_post_event_resolves_transition_and_ignores_unknown_event() -> Result<()> {
    let asking = FnState::new("asking").add_transition(
        "PAUSE",
        Box::new(SimpleTransition::new("PAUSE", "suspended")),
    );

    let suspended = FnState::new("suspended").add_transition(
        "RESUME",
        Box::new(SimpleTransition::new("RESUME", "asking")),
    );

    let sm = StateMachine::builder("pause_resume")
        .add_state(Box::new(asking))
        .add_state(Box::new(suspended))
        .initial_state("asking")
        .build()
        .await?;

    sm.start().await?;
    assert_eq!(sm.current_state(), "asking");

    sm.post_event("PAUSE");
    sm.tick().await?;
    assert_eq!(sm.current_state(), "suspended");

    sm.post_event("UNKNOWN");
    sm.tick().await?;
    assert_eq!(sm.current_state(), "suspended");

    sm.post_event("RESUME");
    sm.tick().await?;
    assert_eq!(sm.current_state(), "asking");
    Ok(())
}
