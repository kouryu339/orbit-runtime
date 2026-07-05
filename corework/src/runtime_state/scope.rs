#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateScope {
    Local {
        unit_id: String,
        cache_scope_id: String,
    },
    Subtree {
        owner_unit_id: String,
    },
    Conversation {
        conversation_id: String,
    },
    Global,
}

impl StateScope {
    pub fn local(unit_id: impl Into<String>, cache_scope_id: impl Into<String>) -> Self {
        Self::Local {
            unit_id: unit_id.into(),
            cache_scope_id: cache_scope_id.into(),
        }
    }

    pub fn subtree(owner_unit_id: impl Into<String>) -> Self {
        Self::Subtree {
            owner_unit_id: owner_unit_id.into(),
        }
    }

    pub fn conversation(conversation_id: impl Into<String>) -> Self {
        Self::Conversation {
            conversation_id: conversation_id.into(),
        }
    }

    pub fn key(&self, key: &str) -> String {
        match self {
            Self::Local { cache_scope_id, .. } => format!("{cache_scope_id}:{key}"),
            Self::Subtree { owner_unit_id } => format!("__subtree__:{owner_unit_id}:{key}"),
            Self::Conversation { conversation_id } => {
                format!("conversation:{conversation_id}:{key}")
            }
            Self::Global => key.to_string(),
        }
    }

    pub fn key_prefix(&self) -> Option<String> {
        match self {
            Self::Local { cache_scope_id, .. } => Some(format!("{cache_scope_id}:")),
            Self::Subtree { owner_unit_id } => Some(format!("__subtree__:{owner_unit_id}:")),
            Self::Conversation { conversation_id } => {
                Some(format!("conversation:{conversation_id}:"))
            }
            Self::Global => None,
        }
    }
}
