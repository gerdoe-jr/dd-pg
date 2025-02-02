use std::{
    collections::VecDeque,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use async_trait::async_trait;
use base::system::SystemTime;
use network::network::{
    connection::NetworkConnectionId, event::NetworkEvent,
    event_generator::NetworkEventToGameEventGenerator,
};
use tokio::sync::Mutex;

use crate::messages::GameMessage;

pub enum GameEvents<'a> {
    NetworkEvent(NetworkEvent),
    NetworkMsg(GameMessage<'a>),
}

pub struct GameEventGenerator {
    pub events: Arc<Mutex<VecDeque<(NetworkConnectionId, Duration, GameEvents<'static>)>>>,
    pub has_events: Arc<AtomicBool>,
}

impl GameEventGenerator {
    pub fn new(has_events: Arc<AtomicBool>, _sys: Arc<SystemTime>) -> Self {
        GameEventGenerator {
            events: Default::default(),
            has_events,
        }
    }
}

#[async_trait]
impl NetworkEventToGameEventGenerator for GameEventGenerator {
    async fn generate_from_binary(
        &self,
        timestamp: Duration,
        con_id: &NetworkConnectionId,
        bytes: &[u8],
    ) {
        let msg =
            bincode::serde::decode_from_slice::<GameMessage, _>(bytes, bincode::config::standard());
        match msg {
            Ok((msg, _)) => {
                self.events.lock().await.push_back((
                    *con_id,
                    timestamp,
                    GameEvents::NetworkMsg(msg),
                ));
                self.has_events
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Err(err) => {
                log::debug!("failed to decode msg {err}");
            }
        }
    }

    async fn generate_from_network_event(
        &self,
        timestamp: Duration,
        con_id: &NetworkConnectionId,
        network_event: &NetworkEvent,
    ) -> bool {
        {
            let mut events = self.events.lock().await;
            // network stats are not vital, so drop them if the queue gets too big
            if !matches!(network_event, NetworkEvent::NetworkStats(_)) || events.len() < 200 {
                events.push_back((
                    *con_id,
                    timestamp,
                    GameEvents::NetworkEvent(network_event.clone()),
                ));
            }
        }
        self.has_events
            .store(true, std::sync::atomic::Ordering::Relaxed);
        true
    }
}
