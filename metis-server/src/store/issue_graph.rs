use std::collections::{HashMap, HashSet, VecDeque};

use super::StoreError;
use metis_common::{
    IssueId,
    issues::{
        Issue, IssueDependencyType, IssueGraphFilter, IssueGraphFilterSide, IssueGraphWildcard,
    },
};
use tracing::warn;

pub(crate) struct IssueGraphContext {
    known_issues: HashSet<IssueId>,
    forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
    reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
}

impl IssueGraphContext {
    pub(crate) fn from_issues(issues: &[(IssueId, Issue)]) -> Self {
        let mut forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        let mut reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();

        for (issue_id, issue) in issues {
            for dependency in &issue.dependencies {
                forward
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(dependency.issue_id.clone())
                    .or_default()
                    .push(issue_id.clone());

                reverse
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(issue_id.clone())
                    .or_default()
                    .push(dependency.issue_id.clone());
            }
        }

        Self {
            known_issues: issues.iter().map(|(id, _)| id.clone()).collect(),
            forward,
            reverse,
        }
    }

    pub(crate) fn from_dependency_maps(
        known_issues: HashSet<IssueId>,
        forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
        reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
    ) -> Self {
        Self {
            known_issues,
            forward,
            reverse,
        }
    }

    pub(crate) fn apply_filters(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        let mut intersection: Option<HashSet<IssueId>> = None;

        for filter in filters {
            let literal = filter.literal_issue_id();
            if !self.known_issues.contains(literal) {
                return Err(StoreError::IssueNotFound(literal.clone()));
            }

            let adjacency = self.adjacency(filter.wildcard_position(), filter.dependency_type);
            let matches = collect_matches(adjacency, literal, filter.wildcard_kind());

            match &mut intersection {
                Some(existing) => existing.retain(|id| matches.contains(id)),
                None => intersection = Some(matches),
            }

            if let Some(existing) = &intersection {
                if existing.is_empty() {
                    break;
                }
            }
        }

        Ok(intersection.unwrap_or_default())
    }

    fn adjacency(
        &self,
        side: IssueGraphFilterSide,
        dependency_type: IssueDependencyType,
    ) -> Option<&HashMap<IssueId, Vec<IssueId>>> {
        match side {
            IssueGraphFilterSide::Left => self.forward.get(&dependency_type),
            IssueGraphFilterSide::Right => self.reverse.get(&dependency_type),
            other => {
                warn!(?other, "unsupported issue graph filter side");
                None
            }
        }
    }
}

fn collect_matches(
    adjacency: Option<&HashMap<IssueId, Vec<IssueId>>>,
    literal: &IssueId,
    wildcard: IssueGraphWildcard,
) -> HashSet<IssueId> {
    let Some(map) = adjacency else {
        return HashSet::new();
    };

    match wildcard {
        IssueGraphWildcard::Immediate => map
            .get(literal)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        IssueGraphWildcard::Transitive => {
            let mut matches = HashSet::new();
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();

            visited.insert(literal.clone());
            queue.push_back(literal.clone());

            while let Some(current) = queue.pop_front() {
                if let Some(neighbors) = map.get(&current) {
                    for neighbor in neighbors {
                        if visited.insert(neighbor.clone()) {
                            queue.push_back(neighbor.clone());
                        }
                        matches.insert(neighbor.clone());
                    }
                }
            }

            matches
        }
        other => {
            warn!(?other, "unsupported issue graph wildcard");
            HashSet::new()
        }
    }
}
