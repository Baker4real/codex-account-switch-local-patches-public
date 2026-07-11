use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{
    account_identity_key, AccountIdentity, UsageFact, ATTRIBUTION_KIND_LEGACY_PROFILE,
    ATTRIBUTION_KIND_STRICT_ACCOUNT, LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED,
    LEDGER_LAYER_STRICT_PROVEN,
};

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
    trusted_canonical_by_evidence: BTreeMap<String, String>,
    latest_prior_by_evidence: BTreeMap<String, String>,
    latest_prior_eligible_evidence: BTreeSet<String>,
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
        let trusted_aliases = facts
            .iter()
            .enumerate()
            .map(|(index, fact)| {
                let strict_aliases = if fact.ledger_layer == LEDGER_LAYER_STRICT_PROVEN
                    && fact.attribution_kind == ATTRIBUTION_KIND_STRICT_ACCOUNT
                {
                    aliases(fact).collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                if strict_aliases.is_empty() {
                    verified_aliases[index]
                        .iter()
                        .flat_map(|value| value.aliases())
                        .collect::<Vec<_>>()
                } else {
                    strict_aliases
                }
            })
            .collect::<Vec<_>>();
        let confirmed_facts = trusted_aliases
            .iter()
            .map(|aliases| !aliases.is_empty())
            .collect::<Vec<_>>();
        let profile_trusted_components = build_trusted_components(facts, &trusted_aliases, true);
        let global_trusted_components = build_trusted_components(facts, &trusted_aliases, false);
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
        for (root, component) in components {
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

        let mut trusted_canonical_by_evidence = BTreeMap::new();
        for (index, fact) in facts.iter().enumerate() {
            let (Some(_profile_trusted), Some(global_trusted), Some(global_canonical)) = (
                profile_trusted_components[index].as_ref(),
                global_trusted_components[index].as_ref(),
                canonical_by_root.get(&roots[index]),
            ) else {
                continue;
            };
            let trusted_canonical = if global_trusted.aliases.contains(global_canonical) {
                global_canonical.clone()
            } else {
                global_trusted.canonical.clone()
            };
            trusted_canonical_by_evidence.insert(fact.evidence_id.clone(), trusted_canonical);
        }

        let mut timelines = BTreeMap::<String, Vec<ConfirmedIdentityPoint>>::new();
        for (index, fact) in facts.iter().enumerate() {
            if !confirmed_facts[index] {
                continue;
            }
            let Some(canonical) = trusted_canonical_by_evidence.get(&fact.evidence_id) else {
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

        let latest_prior_eligible_evidence = facts
            .iter()
            .filter(|fact| is_latest_prior_eligible(fact))
            .map(|fact| fact.evidence_id.clone())
            .collect::<BTreeSet<_>>();
        let mut eligible_events_by_profile = BTreeMap::<String, Vec<EligibleUsageEvent>>::new();
        for fact in facts
            .iter()
            .filter(|fact| latest_prior_eligible_evidence.contains(&fact.evidence_id))
        {
            eligible_events_by_profile
                .entry(fact.profile.clone())
                .or_default()
                .push(EligibleUsageEvent {
                    order: attribution_order(fact),
                    evidence_id: fact.evidence_id.clone(),
                });
        }
        for events in eligible_events_by_profile.values_mut() {
            events.sort_by(|left, right| left.order.cmp(&right.order));
        }

        let mut latest_prior_by_evidence = BTreeMap::new();
        for (profile, events) in eligible_events_by_profile {
            let Some(timeline) = timelines.get(&profile) else {
                continue;
            };
            resolve_latest_priors(timeline, &events, &mut latest_prior_by_evidence);
        }
        for (index, fact) in facts.iter().enumerate() {
            let Some(canonical) = latest_prior_by_evidence.get(&fact.evidence_id) else {
                continue;
            };
            canonical_by_node.insert(
                IdentityNode::Fallback(fallback_keys[index].clone()),
                canonical.clone(),
            );
        }
        Self {
            canonical_by_node,
            trusted_canonical_by_evidence,
            latest_prior_by_evidence,
            latest_prior_eligible_evidence,
        }
    }

    pub(super) fn canonical_key(&self, fact: &UsageFact, fallback: String) -> String {
        if let Some(canonical) = self.trusted_canonical_by_evidence.get(&fact.evidence_id) {
            return canonical.clone();
        }
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
        if self
            .latest_prior_eligible_evidence
            .contains(&fact.evidence_id)
        {
            return raw_identity_independent_fallback(fact);
        }
        self.canonical_by_node
            .get(&IdentityNode::Fallback(fallback.clone()))
            .cloned()
            .unwrap_or(fallback)
    }

    pub(super) fn matches_group_filter(
        &self,
        fact: &UsageFact,
        fallback: String,
        selected_group_key: &str,
    ) -> bool {
        let Some(selected_canonical) = self.canonical_for_group_key(selected_group_key) else {
            return false;
        };
        self.canonical_key(fact, fallback) == selected_canonical
    }

    fn canonical_for_group_key(&self, group_key: &str) -> Option<String> {
        let mut candidates = self
            .canonical_by_node
            .values()
            .filter(|canonical| canonical.as_str() == group_key)
            .cloned()
            .collect::<BTreeSet<_>>();
        candidates.extend(
            IdentityAlias::parse_account_id(Some(group_key))
                .or_else(|| IdentityAlias::parse_email(Some(group_key)))
                .and_then(|alias| self.canonical_by_node.get(&IdentityNode::Alias(alias)))
                .cloned(),
        );
        candidates.extend(
            self.canonical_by_node
                .get(&IdentityNode::Fallback(group_key.to_string()))
                .cloned(),
        );
        for prefix in ["identity:", "legacy:"] {
            candidates.extend(
                self.canonical_by_node
                    .get(&IdentityNode::Fallback(format!("{prefix}{group_key}")))
                    .cloned(),
            );
        }
        (candidates.len() == 1)
            .then(|| candidates.into_iter().next())
            .flatten()
    }
}

fn raw_identity_independent_fallback(fact: &UsageFact) -> String {
    fact.legacy_account_key
        .as_deref()
        .map(|value| format!("legacy:{value}"))
        .unwrap_or_else(|| format!("profile:{}", fact.profile))
}

fn is_latest_prior_eligible(fact: &UsageFact) -> bool {
    fact.ledger_layer == LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED
        && fact.attribution_kind == ATTRIBUTION_KIND_LEGACY_PROFILE
        && aliases(fact).next().is_none()
        && fact.account_identity_key.is_none()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AttributionOrder {
    created_at_ms: u64,
    numeric_window_id: Option<u64>,
    attribution_window_id: Option<String>,
    evidence_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TemporalRelation {
    Prior,
    After,
    Unknown,
}

struct ConfirmedIdentityPoint {
    order: AttributionOrder,
    canonical: String,
}

struct EligibleUsageEvent {
    order: AttributionOrder,
    evidence_id: String,
}

fn resolve_latest_priors(
    timeline: &[ConfirmedIdentityPoint],
    events: &[EligibleUsageEvent],
    latest_prior_by_evidence: &mut BTreeMap<String, String>,
) {
    let mut timeline_index = 0;
    let mut event_index = 0;
    let mut previous_prior = None::<String>;

    while event_index < events.len() {
        let event_time = events[event_index].order.created_at_ms;
        while timeline_index < timeline.len()
            && timeline[timeline_index].order.created_at_ms < event_time
        {
            let cohort_end = cohort_end(timeline, timeline_index);
            if let Some(canonical) =
                ConfirmedIdentityCohort::from_points(&timeline[timeline_index..cohort_end])
                    .completed_candidate
            {
                previous_prior = Some(canonical);
            }
            timeline_index = cohort_end;
        }

        let event_end = event_cohort_end(events, event_index);
        if timeline_index < timeline.len()
            && timeline[timeline_index].order.created_at_ms == event_time
        {
            let cohort_end = cohort_end(timeline, timeline_index);
            let cohort =
                ConfirmedIdentityCohort::from_points(&timeline[timeline_index..cohort_end]);
            for event in &events[event_index..event_end] {
                if let Some(canonical) = cohort
                    .prior_for_event(&event.order)
                    .or(previous_prior.as_deref())
                {
                    latest_prior_by_evidence
                        .insert(event.evidence_id.clone(), canonical.to_string());
                }
            }
            if let Some(canonical) = cohort.completed_candidate {
                previous_prior = Some(canonical);
            }
            timeline_index = cohort_end;
        } else if let Some(canonical) = previous_prior.as_ref() {
            for event in &events[event_index..event_end] {
                latest_prior_by_evidence.insert(event.evidence_id.clone(), canonical.clone());
            }
        }
        event_index = event_end;
    }
}

fn cohort_end(timeline: &[ConfirmedIdentityPoint], start: usize) -> usize {
    let created_at_ms = timeline[start].order.created_at_ms;
    timeline[start..].partition_point(|point| point.order.created_at_ms == created_at_ms) + start
}

fn event_cohort_end(events: &[EligibleUsageEvent], start: usize) -> usize {
    let created_at_ms = events[start].order.created_at_ms;
    events[start..].partition_point(|event| event.order.created_at_ms == created_at_ms) + start
}

struct ConfirmedIdentityCohort {
    numeric_groups: Vec<ConfirmedWindowGroup>,
    missing_window_canonicals: CanonicalSetSummary,
    completed_candidate: Option<String>,
}

struct ConfirmedWindowGroup {
    window_id: u64,
    canonicals: CanonicalSetSummary,
    candidate_after: Option<String>,
}

struct CanonicalSetSummary {
    first: Option<String>,
    has_multiple: bool,
}

impl CanonicalSetSummary {
    fn from_canonicals(canonicals: BTreeSet<String>) -> Self {
        Self {
            first: canonicals.first().cloned(),
            has_multiple: canonicals.len() > 1,
        }
    }

    fn has_competitor(&self, candidate: &str) -> bool {
        self.has_multiple
            || self
                .first
                .as_deref()
                .is_some_and(|value| value != candidate)
    }
}

impl ConfirmedIdentityCohort {
    fn from_points(points: &[ConfirmedIdentityPoint]) -> Self {
        let mut all_canonicals = BTreeSet::new();
        let mut missing_window_canonicals = BTreeSet::new();
        let mut canonicals_by_window = BTreeMap::<u64, BTreeSet<String>>::new();
        for point in points {
            all_canonicals.insert(point.canonical.clone());
            if let Some(window_id) = point.order.numeric_window_id {
                canonicals_by_window
                    .entry(window_id)
                    .or_default()
                    .insert(point.canonical.clone());
            } else {
                missing_window_canonicals.insert(point.canonical.clone());
            }
        }

        let mut prefix_seen = false;
        let mut prefix_uniform = None::<String>;
        let mut numeric_groups = Vec::with_capacity(canonicals_by_window.len());
        for (window_id, canonicals) in canonicals_by_window {
            let group_uniform = single_canonical(&canonicals).cloned();
            prefix_uniform = if !prefix_seen {
                prefix_seen = true;
                group_uniform.clone()
            } else {
                match (prefix_uniform.as_ref(), group_uniform.as_ref()) {
                    (Some(prefix), Some(group)) if prefix == group => Some(prefix.clone()),
                    _ => None,
                }
            };
            let candidate_after = prefix_uniform.clone().or_else(|| group_uniform.clone());
            numeric_groups.push(ConfirmedWindowGroup {
                window_id,
                canonicals: CanonicalSetSummary::from_canonicals(canonicals),
                candidate_after,
            });
        }

        let completed_candidate = single_canonical(&all_canonicals).cloned().or_else(|| {
            missing_window_canonicals
                .is_empty()
                .then(|| {
                    numeric_groups
                        .last()
                        .and_then(|group| group.candidate_after.clone())
                })
                .flatten()
        });
        Self {
            numeric_groups,
            missing_window_canonicals: CanonicalSetSummary::from_canonicals(
                missing_window_canonicals,
            ),
            completed_candidate,
        }
    }

    fn prior_for_event(&self, event: &AttributionOrder) -> Option<&str> {
        let prior_group_count = self.numeric_groups.partition_point(|group| {
            same_millisecond_relation(Some(group.window_id), event.numeric_window_id)
                == TemporalRelation::Prior
        });
        let candidate = self
            .numeric_groups
            .get(prior_group_count.checked_sub(1)?)?
            .candidate_after
            .as_deref()?;
        if same_millisecond_relation(None, event.numeric_window_id) == TemporalRelation::Unknown
            && self.missing_window_canonicals.has_competitor(candidate)
        {
            return None;
        }
        if self
            .numeric_groups
            .get(prior_group_count)
            .filter(|group| {
                same_millisecond_relation(Some(group.window_id), event.numeric_window_id)
                    == TemporalRelation::Unknown
            })
            .is_some_and(|group| group.canonicals.has_competitor(candidate))
        {
            return None;
        }
        Some(candidate)
    }
}

fn same_millisecond_relation(
    confirmed_window: Option<u64>,
    event_window: Option<u64>,
) -> TemporalRelation {
    match (confirmed_window, event_window) {
        (Some(confirmed), Some(event)) if confirmed < event => TemporalRelation::Prior,
        (Some(confirmed), Some(event)) if confirmed > event => TemporalRelation::After,
        _ => TemporalRelation::Unknown,
    }
}

fn single_canonical(canonicals: &BTreeSet<String>) -> Option<&String> {
    (canonicals.len() == 1)
        .then(|| canonicals.first())
        .flatten()
}

fn attribution_order(fact: &UsageFact) -> AttributionOrder {
    // `created_at_ms` has millisecond precision, so equal timestamps require two
    // numeric persisted windows to prove order. The raw window and evidence id
    // only stabilize sorting; their lexical order is never attribution proof.
    AttributionOrder {
        created_at_ms: fact.created_at_ms,
        numeric_window_id: fact
            .attribution_window_id
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok()),
        attribution_window_id: fact.attribution_window_id.clone(),
        evidence_id: fact.evidence_id.clone(),
    }
}

#[derive(Clone)]
struct TrustedIdentityComponent {
    canonical: String,
    aliases: BTreeSet<String>,
}

fn build_trusted_components(
    facts: &[UsageFact],
    aliases_by_fact: &[Vec<IdentityAlias>],
    profile_scoped: bool,
) -> Vec<Option<TrustedIdentityComponent>> {
    let mut parents = (0..facts.len()).collect::<Vec<_>>();
    let mut owner_by_alias = BTreeMap::<(Option<String>, IdentityAlias), usize>::new();
    for (index, (fact, fact_aliases)) in facts.iter().zip(aliases_by_fact).enumerate() {
        let scope = profile_scoped.then(|| fact.profile.clone());
        for alias in fact_aliases {
            if let Some(owner) = owner_by_alias.insert((scope.clone(), alias.clone()), index) {
                union(&mut parents, index, owner);
            }
        }
    }

    let roots = (0..facts.len())
        .map(|index| find(&mut parents, index))
        .collect::<Vec<_>>();
    let mut components = BTreeMap::<usize, ComponentAliases>::new();
    for (index, fact_aliases) in aliases_by_fact.iter().enumerate() {
        if fact_aliases.is_empty() {
            continue;
        }
        let component = components.entry(roots[index]).or_default();
        for alias in fact_aliases {
            match alias {
                IdentityAlias::AccountId(value) => {
                    component.account_ids.insert(value.clone());
                }
                IdentityAlias::Email(value) => {
                    component.emails.insert(value.clone());
                }
            }
        }
    }

    let components = components
        .into_iter()
        .filter_map(|(root, component)| {
            let canonical = component
                .account_ids
                .first()
                .or_else(|| component.emails.first())?
                .clone();
            let aliases = component
                .account_ids
                .into_iter()
                .chain(component.emails)
                .collect();
            Some((root, TrustedIdentityComponent { canonical, aliases }))
        })
        .collect::<BTreeMap<_, _>>();
    roots
        .into_iter()
        .enumerate()
        .map(|(index, root)| {
            (!aliases_by_fact[index].is_empty())
                .then(|| components.get(&root).cloned())
                .flatten()
        })
        .collect()
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
