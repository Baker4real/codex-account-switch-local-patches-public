use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{account_identity_key, AccountIdentity, UsageFact};

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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IdentityAlias {
    AccountId(String),
    Email(String),
}

impl IdentityAlias {
    fn parse_account_id(value: Option<&str>) -> Option<Self> {
        parse_persisted_alias(value, "acct:").map(Self::AccountId)
    }

    fn parse_email(value: Option<&str>) -> Option<Self> {
        parse_persisted_alias(value, "email:").map(Self::Email)
    }

    fn into_key(self) -> String {
        match self {
            Self::AccountId(value) | Self::Email(value) => value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IdentityNode {
    Alias(IdentityAlias),
    Fallback(String),
}

fn parse_persisted_alias(value: Option<&str>, prefix: &str) -> Option<String> {
    let value = value?;
    let digest = value.strip_prefix(prefix)?;
    (digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
    .then(|| value.to_string())
}

pub(super) fn validated_account_id_alias(value: Option<&str>) -> Option<String> {
    IdentityAlias::parse_account_id(value).map(IdentityAlias::into_key)
}

pub(super) fn validated_email_alias(value: Option<&str>) -> Option<String> {
    IdentityAlias::parse_email(value).map(IdentityAlias::into_key)
}

pub(super) struct CanonicalAccountResolver {
    canonical_by_node: BTreeMap<IdentityNode, String>,
}

impl CanonicalAccountResolver {
    pub(super) fn from_facts(facts: &[UsageFact], fallback_keys: &[String]) -> Self {
        debug_assert_eq!(facts.len(), fallback_keys.len());
        let mut parents = (0..facts.len()).collect::<Vec<_>>();
        let mut owner_by_node = BTreeMap::<IdentityNode, usize>::new();
        for (index, (fact, fallback_key)) in facts.iter().zip(fallback_keys).enumerate() {
            for node in identity_nodes(fact, fallback_key) {
                if let Some(owner) = owner_by_node.insert(node, index) {
                    union(&mut parents, index, owner);
                }
            }
        }
        for (index, fact) in facts.iter().enumerate() {
            if aliases(fact).next().is_some() || fact.account_identity_key.is_none() {
                continue;
            }
            let Some(label) = fact
                .account_label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
            else {
                continue;
            };
            let (candidate_identity, candidate_alias) = if label.contains('@') {
                (
                    AccountIdentity {
                        account_id: None,
                        email: Some(label.to_string()),
                    },
                    email_alias(Some(label)).map(IdentityAlias::Email),
                )
            } else {
                (
                    AccountIdentity {
                        account_id: Some(label.to_string()),
                        email: None,
                    },
                    account_id_alias(Some(label)).map(IdentityAlias::AccountId),
                )
            };
            let candidate_identity_key = account_identity_key(&candidate_identity);
            if fact.account_identity_key.as_deref() != Some(candidate_identity_key.as_str()) {
                continue;
            }
            let Some(candidate_alias) = candidate_alias else {
                continue;
            };
            if let Some(owner) = owner_by_node.get(&IdentityNode::Alias(candidate_alias)) {
                union(&mut parents, index, *owner);
            }
        }

        let mut components = BTreeMap::<usize, ComponentAliases>::new();
        for (index, fact) in facts.iter().enumerate() {
            let root = find(&mut parents, index);
            let component = components.entry(root).or_default();
            for alias in aliases(fact) {
                match alias {
                    IdentityAlias::AccountId(value) => {
                        component.account_ids.insert(value);
                    }
                    IdentityAlias::Email(value) => {
                        component.emails.insert(value);
                    }
                }
            }
            component.fallbacks.insert(fallback_keys[index].clone());
        }

        let mut canonical_by_node = BTreeMap::new();
        for component in components.into_values() {
            let canonical = component
                .account_ids
                .first()
                .or_else(|| component.emails.first())
                .or_else(|| component.fallbacks.first())
                .cloned();
            let Some(canonical) = canonical else {
                continue;
            };
            for node in component
                .account_ids
                .into_iter()
                .map(|value| IdentityNode::Alias(IdentityAlias::AccountId(value)))
                .chain(
                    component
                        .emails
                        .into_iter()
                        .map(|value| IdentityNode::Alias(IdentityAlias::Email(value))),
                )
                .chain(component.fallbacks.into_iter().map(IdentityNode::Fallback))
            {
                canonical_by_node.insert(node, canonical.clone());
            }
        }
        Self { canonical_by_node }
    }

    pub(super) fn canonical_key(&self, fact: &UsageFact, fallback: String) -> String {
        for alias in aliases(fact) {
            if let Some(canonical) = self
                .canonical_by_node
                .get(&IdentityNode::Alias(alias))
                .cloned()
            {
                return canonical;
            }
        }
        self.canonical_by_node
            .get(&IdentityNode::Fallback(fallback.clone()))
            .cloned()
            .unwrap_or(fallback)
    }
}

#[derive(Default)]
struct ComponentAliases {
    account_ids: BTreeSet<String>,
    emails: BTreeSet<String>,
    fallbacks: BTreeSet<String>,
}

fn identity_nodes<'a>(
    fact: &'a UsageFact,
    fallback: &str,
) -> impl Iterator<Item = IdentityNode> + 'a {
    aliases(fact)
        .map(IdentityNode::Alias)
        .chain(std::iter::once(IdentityNode::Fallback(
            fallback.to_string(),
        )))
}

fn aliases(fact: &UsageFact) -> impl Iterator<Item = IdentityAlias> + '_ {
    [
        IdentityAlias::parse_account_id(fact.account_id_alias.as_deref()),
        IdentityAlias::parse_email(fact.email_alias.as_deref()),
    ]
    .into_iter()
    .flatten()
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
