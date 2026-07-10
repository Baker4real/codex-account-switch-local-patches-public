use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::UsageFact;

pub(super) fn account_id_alias(value: Option<&str>) -> Option<String> {
    normalized_non_empty(value).map(|value| format!("acct:{}", sha256_hex(value.as_bytes())))
}

pub(super) fn email_alias(value: Option<&str>) -> Option<String> {
    normalized_non_empty(value)
        .map(str::to_ascii_lowercase)
        .map(|value| format!("email:{}", sha256_hex(value.as_bytes())))
}

fn normalized_non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

pub(super) struct CanonicalAccountResolver {
    canonical_by_alias: BTreeMap<String, String>,
}

impl CanonicalAccountResolver {
    pub(super) fn from_facts(facts: &[UsageFact]) -> Self {
        let mut parents = (0..facts.len()).collect::<Vec<_>>();
        let mut owner_by_alias = BTreeMap::<String, usize>::new();
        for (index, fact) in facts.iter().enumerate() {
            for alias in aliases(fact) {
                if let Some(owner) = owner_by_alias.insert(alias.to_string(), index) {
                    union(&mut parents, index, owner);
                }
            }
        }

        let mut components = BTreeMap::<usize, ComponentAliases>::new();
        for (index, fact) in facts.iter().enumerate() {
            let root = find(&mut parents, index);
            let component = components.entry(root).or_default();
            if let Some(alias) = non_empty_alias(fact.account_id_alias.as_deref()) {
                component.account_ids.insert(alias.to_string());
            }
            if let Some(alias) = non_empty_alias(fact.email_alias.as_deref()) {
                component.emails.insert(alias.to_string());
            }
        }

        let mut canonical_by_alias = BTreeMap::new();
        for component in components.into_values() {
            let canonical = component
                .account_ids
                .first()
                .or_else(|| component.emails.first())
                .cloned();
            let Some(canonical) = canonical else {
                continue;
            };
            for alias in component.account_ids.into_iter().chain(component.emails) {
                canonical_by_alias.insert(alias, canonical.clone());
            }
        }
        Self { canonical_by_alias }
    }

    pub(super) fn canonical_key(&self, fact: &UsageFact, fallback: String) -> String {
        aliases(fact)
            .find_map(|alias| self.canonical_by_alias.get(alias).cloned())
            .unwrap_or(fallback)
    }
}

#[derive(Default)]
struct ComponentAliases {
    account_ids: BTreeSet<String>,
    emails: BTreeSet<String>,
}

fn aliases(fact: &UsageFact) -> impl Iterator<Item = &str> {
    [
        non_empty_alias(fact.account_id_alias.as_deref()),
        non_empty_alias(fact.email_alias.as_deref()),
    ]
    .into_iter()
    .flatten()
}

fn non_empty_alias(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn find(parents: &mut [usize], index: usize) -> usize {
    if parents[index] != index {
        let parent = parents[index];
        parents[index] = find(parents, parent);
    }
    parents[index]
}

fn union(parents: &mut [usize], left: usize, right: usize) {
    let left_root = find(parents, left);
    let right_root = find(parents, right);
    if left_root != right_root {
        parents[right_root] = left_root;
    }
}
