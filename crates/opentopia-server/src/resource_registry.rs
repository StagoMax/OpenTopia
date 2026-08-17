use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// Storage identity captured when a View resource is opened. Paths are never
/// encoded into resource IDs; the registry binds each opaque ID to one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResourceLocator {
    Workspace { path: PathBuf },
    Local { path: PathBuf },
    Artifact { artifact_id: Uuid },
    Attachment { attachment_id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResourceLease {
    pub(super) id: Uuid,
    pub(super) thread_id: Uuid,
    pub(super) locator: ResourceLocator,
}

#[derive(Clone, Default)]
pub(super) struct ResourceRegistry {
    leases: Arc<RwLock<HashMap<Uuid, ResourceLease>>>,
}

impl ResourceRegistry {
    pub(super) fn register(&self, thread_id: Uuid, locator: ResourceLocator) -> ResourceLease {
        let lease = ResourceLease {
            id: Uuid::new_v4(),
            thread_id,
            locator,
        };
        self.leases
            .write()
            .expect("resource registry poisoned")
            .insert(lease.id, lease.clone());
        lease
    }

    pub(super) fn replace_locator(&self, id: Uuid, locator: ResourceLocator) {
        if let Some(lease) = self
            .leases
            .write()
            .expect("resource registry poisoned")
            .get_mut(&id)
        {
            lease.locator = locator;
        }
    }

    pub(super) fn get(&self, thread_id: Uuid, id: Uuid) -> Option<ResourceLease> {
        self.leases
            .read()
            .expect("resource registry poisoned")
            .get(&id)
            .filter(|lease| lease.thread_id == thread_id)
            .cloned()
    }

    pub(super) fn release(&self, thread_id: Uuid, id: Uuid) -> bool {
        let mut leases = self.leases.write().expect("resource registry poisoned");
        if leases
            .get(&id)
            .is_some_and(|lease| lease.thread_id == thread_id)
        {
            leases.remove(&id);
            true
        } else {
            false
        }
    }

    pub(super) fn release_thread(&self, thread_id: Uuid) -> usize {
        let mut leases = self.leases.write().expect("resource registry poisoned");
        let before = leases.len();
        leases.retain(|_, lease| lease.thread_id != thread_id);
        before - leases.len()
    }
}

pub(super) fn resource_preview_id(id: Uuid) -> String {
    format!("resource.{id}")
}

pub(super) fn parse_resource_preview_id(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value.strip_prefix("resource.")?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_are_opaque_and_task_scoped() {
        let registry = ResourceRegistry::default();
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        let lease = registry.register(
            owner,
            ResourceLocator::Local {
                path: PathBuf::from("C:/private/说明.md"),
            },
        );

        let id = resource_preview_id(lease.id);
        assert!(!id.contains("private"));
        assert_eq!(parse_resource_preview_id(&id), Some(lease.id));
        assert!(registry.get(owner, lease.id).is_some());
        assert!(registry.get(other, lease.id).is_none());
        assert!(!registry.release(other, lease.id));
        assert!(registry.release(owner, lease.id));
    }

    #[test]
    fn deleting_a_task_releases_all_of_its_resource_leases() {
        let registry = ResourceRegistry::default();
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        registry.register(
            owner,
            ResourceLocator::Workspace {
                path: PathBuf::from("one.md"),
            },
        );
        registry.register(
            owner,
            ResourceLocator::Workspace {
                path: PathBuf::from("two.md"),
            },
        );
        let retained = registry.register(
            other,
            ResourceLocator::Workspace {
                path: PathBuf::from("other.md"),
            },
        );

        assert_eq!(registry.release_thread(owner), 2);
        assert!(registry.get(other, retained.id).is_some());
    }
}
