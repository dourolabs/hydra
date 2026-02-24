use metis_common::ActorId;

/// Canonical conversation identifier built from an unordered pair of actors.
///
/// The two actor name strings are sorted lexicographically and joined with `+`.
/// For example: `a-i-abcdef+u-alice`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(String);

impl ConversationId {
    /// Build the canonical conversation ID from two actors.
    ///
    /// The actor names are sorted lexicographically so that the conversation
    /// between A and B is the same regardless of who initiates it.
    pub fn from_pair(a: &ActorId, b: &ActorId) -> Self {
        let name_a = actor_id_to_name(a);
        let name_b = actor_id_to_name(b);
        let (first, second) = if name_a <= name_b {
            (name_a, name_b)
        } else {
            (name_b, name_a)
        };
        Self(format!("{first}+{second}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ConversationId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Convert an ActorId to its canonical actor name string.
///
/// This mirrors the logic in `Actor::name()` from the actors module.
fn actor_id_to_name(actor_id: &ActorId) -> String {
    match actor_id {
        ActorId::Username(username) => format!("u-{username}"),
        ActorId::Task(task_id) => format!("w-{task_id}"),
        ActorId::Issue(issue_id) => format!("a-{issue_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::IssueId;
    use metis_common::api::v1::users::Username;
    use std::str::FromStr;

    #[test]
    fn from_pair_produces_canonical_sorted_string() {
        let user = ActorId::Username(Username::from("alice"));
        let issue = ActorId::Issue(IssueId::from_str("i-abcdef").unwrap());

        let conv = ConversationId::from_pair(&user, &issue);
        assert_eq!(conv.as_str(), "a-i-abcdef+u-alice");
    }

    #[test]
    fn from_pair_is_order_independent() {
        let user = ActorId::Username(Username::from("alice"));
        let issue = ActorId::Issue(IssueId::from_str("i-abcdef").unwrap());

        let conv_a = ConversationId::from_pair(&user, &issue);
        let conv_b = ConversationId::from_pair(&issue, &user);
        assert_eq!(conv_a, conv_b);
    }

    #[test]
    fn from_pair_two_issues() {
        let issue_a = ActorId::Issue(IssueId::from_str("i-abcdef").unwrap());
        let issue_b = ActorId::Issue(IssueId::from_str("i-ghijkl").unwrap());

        let conv = ConversationId::from_pair(&issue_a, &issue_b);
        assert_eq!(conv.as_str(), "a-i-abcdef+a-i-ghijkl");
    }

    #[test]
    fn from_pair_two_users() {
        let user_a = ActorId::Username(Username::from("alice"));
        let user_b = ActorId::Username(Username::from("bob"));

        let conv = ConversationId::from_pair(&user_a, &user_b);
        assert_eq!(conv.as_str(), "u-alice+u-bob");
    }
}
