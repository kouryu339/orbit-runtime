//! Generated Corework Agent Tool Protocol bindings.

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/corework.agent_tool.v1.rs"));
}

pub mod events_v1 {
    include!(concat!(env!("OUT_DIR"), "/corework.event_source.v1.rs"));
}
