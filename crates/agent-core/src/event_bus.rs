//! Event bus for inter-component communication.
//!
//! Built on `tokio::broadcast`, supports typed events, filtered subscriptions,
//! and async consumption. Ported from netsec-events with agent-specific types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Default capacity of the broadcast channel.
const DEFAULT_CAPACITY: usize = 1024;

/// Agent event types for the event bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventType {
    SessionStarted,
    SessionEnded,
    MessageSent,
    MessageReceived,
    ToolInvoked,
    ToolCompleted,
    ProviderSwitched,
    ProviderFailed,
    ScheduleFired,
    ProfileChanged,
    SecretDetected,
    Error,
}

/// A single event published on the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: Uuid,
    pub event_type: AgentEventType,
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

impl AgentEvent {
    /// Create a new event with the given type and payload.
    pub fn new(event_type: AgentEventType, payload: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type,
            payload,
            timestamp: Utc::now(),
        }
    }
}

/// Central event bus for the agent platform.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AgentEvent>,
}

impl EventBus {
    /// Create a new event bus with default capacity.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_CAPACITY);
        Self { sender }
    }

    /// Create a new event bus with a custom channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event to all subscribers.
    pub fn publish(
        &self,
        event: AgentEvent,
    ) -> Result<usize, broadcast::error::SendError<AgentEvent>> {
        self.sender.send(event)
    }

    /// Subscribe to all events.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.sender.subscribe()
    }

    /// Subscribe to only specific event types.
    pub fn subscribe_filtered(&self, types: Vec<AgentEventType>) -> FilteredSubscriber {
        FilteredSubscriber {
            receiver: self.sender.subscribe(),
            filter: types.into_iter().collect(),
        }
    }

    /// Return the number of active subscribers on the channel.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// A subscriber that only yields events matching a set of [`AgentEventType`]s.
pub struct FilteredSubscriber {
    receiver: broadcast::Receiver<AgentEvent>,
    filter: HashSet<AgentEventType>,
}

impl FilteredSubscriber {
    /// Receive the next event that matches the filter.
    ///
    /// Events that do not match are silently skipped.
    pub async fn recv(&mut self) -> Result<AgentEvent, broadcast::error::RecvError> {
        loop {
            let event = self.receiver.recv().await?;
            if self.filter.contains(&event.event_type) {
                return Ok(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_receive_roundtrip() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let event = AgentEvent::new(
            AgentEventType::SessionStarted,
            serde_json::json!({"session_id": "s1"}),
        );
        let event_id = event.id;
        bus.publish(event).unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.event_type, AgentEventType::SessionStarted);
        assert_eq!(received.id, event_id);
    }

    #[tokio::test]
    async fn test_filtered_subscriber_receives_matching() {
        let bus = EventBus::new();
        let mut filtered = bus.subscribe_filtered(vec![AgentEventType::ToolCompleted]);

        let event = AgentEvent::new(
            AgentEventType::ToolCompleted,
            serde_json::json!({"tool": "shell", "exit_code": 0}),
        );
        bus.publish(event).unwrap();

        let received = filtered.recv().await.unwrap();
        assert_eq!(received.event_type, AgentEventType::ToolCompleted);
    }

    #[tokio::test]
    async fn test_filtered_subscriber_skips_non_matching() {
        let bus = EventBus::new();
        let mut filtered = bus.subscribe_filtered(vec![AgentEventType::Error]);

        // Publish non-matching then matching.
        bus.publish(AgentEvent::new(
            AgentEventType::MessageSent,
            serde_json::json!({}),
        ))
        .unwrap();
        let error_event =
            AgentEvent::new(AgentEventType::Error, serde_json::json!({"msg": "timeout"}));
        let error_id = error_event.id;
        bus.publish(error_event).unwrap();

        let received = filtered.recv().await.unwrap();
        assert_eq!(received.event_type, AgentEventType::Error);
        assert_eq!(received.id, error_id);
    }

    #[test]
    fn test_subscriber_count() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);

        let _rx1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);

        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        drop(_rx1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn test_custom_capacity() {
        let bus = EventBus::with_capacity(2);
        let mut rx = bus.subscribe();

        bus.publish(AgentEvent::new(
            AgentEventType::ScheduleFired,
            serde_json::json!({}),
        ))
        .unwrap();
        bus.publish(AgentEvent::new(
            AgentEventType::ProfileChanged,
            serde_json::json!({}),
        ))
        .unwrap();

        let e1 = rx.recv().await.unwrap();
        assert_eq!(e1.event_type, AgentEventType::ScheduleFired);
        let e2 = rx.recv().await.unwrap();
        assert_eq!(e2.event_type, AgentEventType::ProfileChanged);
    }

    #[test]
    fn test_event_serialization() {
        let event = AgentEvent::new(
            AgentEventType::SecretDetected,
            serde_json::json!({"line": 42}),
        );
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, event.id);
        assert_eq!(parsed.event_type, AgentEventType::SecretDetected);
    }
}
