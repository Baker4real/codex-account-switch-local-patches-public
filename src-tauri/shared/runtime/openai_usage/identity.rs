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
    latest_prior_by_evidence: BTreeMap<String, String>,
}

pub(super) type VerifiedProfileAliases = BTreeMap<(String, String), VerifiedIdentityAliases>;

pub(super) struct VerifiedIdentityAliases {
    account_id: IdentityAlias,
    email: IdentityAlias,
}

pub(super) fn verified_identity_aliases(
    identity: &AccountIdentity,
) -> Option<(String, VerifiedIdentityAliases)> {
    let account_id =
        account_id_alias(identity.account_id.as_deref()).map(IdentityAlias::AccountId)?;
    let email = email_alias(identity.email.as_deref()).map(IdentityAlias::Email)?;
    Some((
        account_identity_key(identity),
        VerifiedIdentityAliases { account_id, email },
    ))
}

impl VerifiedIdentityAliases {
    fn aliases(&self) -> impl Iterator<Item = IdentityAlias> + '_ {
        [self.account_id.clone(), self.email.clone()].into_iter()
    }
}

impl CanonicalAccountResolver {
    pub(super) fn from_facts(
        facts: &[UsageFact],
        fallback_keys: &[String],
        verified_profile_aliases: &VerifiedProfileAliases,
    ) -> Self {
        debug_assert_eq!(facts.len(), fallback_keys.len());
        let verified_aliases = facts
            .iter()
            .map(|fact| verified_aliases_for_fact(fact, verified_profile_aliases))
            .collect::<Vec<_>>();
        let confirmed_facts = facts
            .iter()
            .enumerate()
            .map(|(index, fact)| {
                aliases(fact).next().is_some() || verified_aliases[index].is_some()
            })
            .collect::<Vec<_>>();
        let mut parents = (0..facts.len()).collect::<Vec<_>>();
        let mut owner_by_node = BTreeMap::<IdentityNode, usize>::new();
        for (index, (fact, fallback_key)) in facts.iter().zip(fallback_keys).enumerate() {
            let recovered = verified_aliases[index]
                .iter()
                .flat_map(|value| value.aliases());
            for node in identity_nodes(fact, fallback_key, recovered) {
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
            // Legacy v1 verifier: `account_identity_key` is compatibility-locked by
            // published golden vectors before it may prove an alias-less history row.
            let legacy_v1_candidate_key = account_identity_key(&candidate_identity);
            if fact.account_identity_key.as_deref() != Some(legacy_v1_candidate_key.as_str()) {
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
        let mut roots = Vec::with_capacity(facts.len());
        for (index, fact) in facts.iter().enumerate() {
            let root = find(&mut parents, index);
            roots.push(root);
            let component = components.entry(root).or_default();
            for alias in aliases(fact).chain(
                verified_aliases[index]
                    .iter()
                    .flat_map(|value| value.aliases()),
            ) {
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
        let mut canonical_by_root = BTreeMap::new();
        let mut confirmed_roots = BTreeSet::new();
        for (root, component) in components {
            let confirmed = !component.account_ids.is_empty() || !component.emails.is_empty();
            let canonical = component
                .account_ids
                .first()
                .or_else(|| component.emails.first())
                .or_else(|| component.fallbacks.first())
                .cloned();
            let Some(canonical) = canonical else {
                continue;
            };
            canonical_by_root.insert(root, canonical.clone());
            if confirmed {
                confirmed_roots.insert(root);
            }
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

        let mut timelines = BTreeMap::<String, Vec<ConfirmedIdentityPoint>>::new();
        for (index, fact) in facts.iter().enumerate() {
            if !confirmed_facts[index] {
                continue;
            }
            let Some(canonical) = canonical_by_root.get(&roots[index]) else {
                continue;
            };
            timelines
                .entry(fact.profile.clone())
                .or_default()
                .push(ConfirmedIdentityPoint {
                    order: attribution_order(fact),
                    canonical: canonical.clone(),
                });
        }
        for timeline in timelines.values_mut() {
            timeline.sort_by(|left, right| left.order.cmp(&right.order));
        }

        let mut latest_prior_by_evidence = BTreeMap::new();
        for (index, fact) in facts.iter().enumerate() {
            if confirmed_facts[index] || confirmed_roots.contains(&roots[index]) {
                continue;
            }
            let Some(timeline) = timelines.get(&fact.profile) else {
                continue;
            };
            let event_order = attribution_order(fact);
            if let Some(point) = timeline
                .iter()
                .rev()
                .find(|point| point.order < event_order)
            {
                latest_prior_by_evidence.insert(fact.evidence_id.clone(), point.canonical.clone());
            }
        }
        Self {
            canonical_by_node,
            latest_prior_by_evidence,
        }
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
        if let Some(canonical) = self.latest_prior_by_evidence.get(&fact.evidence_id) {
            return canonical.clone();
        }
        self.canonical_by_node
            .get(&IdentityNode::Fallback(fallback.clone()))
            .cloned()
            .unwrap_or(fallback)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AttributionOrder {
    created_at_ms: u64,
    attribution_window_id: Option<String>,
    evidence_id: String,
}

struct ConfirmedIdentityPoint {
    order: AttributionOrder,
    canonical: String,
}

fn attribution_order(fact: &UsageFact) -> AttributionOrder {
    // `created_at_ms` has millisecond precision, so equal timestamps do not prove
    // observation order. Persisted window/evidence ids are the conservative,
    // deterministic sequence; request/load order and current profile state are not.
    AttributionOrder {
        created_at_ms: fact.created_at_ms,
        attribution_window_id: fact.attribution_window_id.clone(),
        evidence_id: fact.evidence_id.clone(),
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
    recovered: impl Iterator<Item = IdentityAlias> + 'a,
) -> impl Iterator<Item = IdentityNode> + 'a {
    aliases(fact)
        .chain(recovered)
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

fn verified_aliases_for_fact<'a>(
    fact: &UsageFact,
    verified_profile_aliases: &'a VerifiedProfileAliases,
) -> Option<&'a VerifiedIdentityAliases> {
    if aliases(fact).next().is_some() {
        return None;
    }
    let identity_key = fact.account_identity_key.as_deref()?;
    verified_profile_aliases.get(&(fact.profile.clone(), identity_key.to_string()))
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
