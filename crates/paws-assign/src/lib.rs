//! `paws assign`: assigns a GitHub issue or pull request to the people who
//! own it, read from the repository's `CODEOWNERS` file (or an explicit
//! list).
//!
//! GitHub has no repository setting for default assignees, and `CODEOWNERS`
//! on its own only requests *reviews* on pull requests. This fills both gaps:
//! a pull request goes to the owners of the files it changes, and an issue
//! (which has no files) goes to the owners of the catch-all `*` rule.
//!
//! Matching and planning are pure functions so they can be tested without a
//! live API; [`GitHubAssignClient`] is the only part that talks to GitHub.

use anyhow::{Context, Result, bail};

/// Where GitHub looks for `CODEOWNERS`, in the order it looks.
pub const CODEOWNERS_PATHS: &[&str] = &[".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];

/// GitHub rejects more than this many assignees on one issue.
pub const MAX_ASSIGNEES: usize = 10;

/// One non-comment line of a `CODEOWNERS` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub pattern: String,
    /// Verbatim owner tokens: `@user`, `@org/team`, or an email address.
    /// Empty means the rule deliberately leaves its paths unowned.
    pub owners: Vec<String>,
}

/// Parses `CODEOWNERS` text into rules, in file order (order matters: the
/// last matching rule wins).
pub fn parse_codeowners(text: &str) -> Vec<Rule> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut tokens = line.split_whitespace();
            let pattern = tokens.next()?.to_string();
            let owners = tokens
                .take_while(|token| !token.starts_with('#'))
                .map(str::to_string)
                .collect();
            Some(Rule { pattern, owners })
        })
        .collect()
}

/// Whether a `CODEOWNERS` pattern matches a repo-relative file path, using
/// the gitignore-style rules GitHub documents: a pattern containing a `/`
/// (other than a trailing one) is anchored to the repo root, otherwise it
/// matches at any depth; a trailing `/` only matches directories; `*` and `?`
/// stay within one path segment and `**` spans any number of them. A pattern
/// that names a directory owns everything under it.
pub fn pattern_matches(pattern: &str, path: &str) -> bool {
    let dir_only = pattern.ends_with('/');
    let trimmed = pattern.trim_end_matches('/');
    let anchored = trimmed.contains('/');
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() {
        return false;
    }

    let mut segments: Vec<&str> = Vec::new();
    if !anchored {
        segments.push("**");
    }
    segments.extend(trimmed.split('/'));
    let path: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    segments_match(&segments, &path, dir_only)
}

fn segments_match(pattern: &[&str], path: &[&str], dir_only: bool) -> bool {
    match pattern.split_first() {
        // Pattern used up: an exact match names the file itself, a leftover
        // path means the pattern named one of its parent directories.
        None => !path.is_empty() || !dir_only,
        Some((&"**", rest)) => {
            (0..=path.len()).any(|skip| segments_match(rest, &path[skip..], dir_only))
        }
        Some((segment, rest)) => path.split_first().is_some_and(|(name, path_rest)| {
            glob_segment(segment, name) && segments_match(rest, path_rest, dir_only)
        }),
    }
}

/// `*`/`?`/`\`-escape matching within a single path segment.
fn glob_segment(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    glob_chars(&pattern, &name)
}

fn glob_chars(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| glob_chars(rest, &name[skip..])),
        Some(('?', rest)) => !name.is_empty() && glob_chars(rest, &name[1..]),
        Some(('\\', rest)) if !rest.is_empty() => {
            name.first() == rest.first() && glob_chars(&rest[1..], &name[1..])
        }
        Some((c, rest)) => name.first() == Some(c) && glob_chars(rest, &name[1..]),
    }
}

/// The owners of every path, in first-seen order without duplicates. Each
/// path takes the owners of the *last* rule matching it, as GitHub does.
pub fn owners_for_paths<S: AsRef<str>>(rules: &[Rule], paths: &[S]) -> Vec<String> {
    let mut owners = Vec::new();
    for path in paths {
        if let Some(rule) = rules
            .iter()
            .rev()
            .find(|rule| pattern_matches(&rule.pattern, path.as_ref()))
        {
            push_unique(&mut owners, &rule.owners);
        }
    }
    owners
}

/// The owners of the repository as a whole: the last catch-all rule (`*`,
/// `**`, `/*`, or `/**`). This is who an issue goes to, since an issue has no
/// files to match.
pub fn default_owners(rules: &[Rule]) -> Vec<String> {
    rules
        .iter()
        .rev()
        .find(|rule| matches!(rule.pattern.as_str(), "*" | "**" | "/*" | "/**"))
        .map(|rule| rule.owners.clone())
        .unwrap_or_default()
}

fn push_unique(into: &mut Vec<String>, items: &[String]) {
    for item in items {
        if !into.contains(item) {
            into.push(item.clone());
        }
    }
}

/// Splits owner tokens into user logins GitHub can assign and the tokens it
/// can't (teams and email addresses are valid owners but not assignees).
pub fn assignable_logins(owners: &[String]) -> (Vec<String>, Vec<String>) {
    let mut logins = Vec::new();
    let mut skipped = Vec::new();
    for owner in owners {
        match owner.strip_prefix('@') {
            Some(login) if !login.is_empty() && !login.contains('/') => {
                push_unique(&mut logins, &[login.to_string()]);
            }
            _ => skipped.push(owner.clone()),
        }
    }
    (logins, skipped)
}

/// The issue or pull request number a GitHub Actions event payload is about
/// (`issues`, `issue_comment`, `pull_request`, `pull_request_target`, ...).
pub fn number_from_event(event: &serde_json::Value) -> Option<u64> {
    event
        .pointer("/issue/number")
        .or_else(|| event.pointer("/pull_request/number"))
        .or_else(|| event.get("number"))
        .and_then(serde_json::Value::as_u64)
}

/// What [`plan`] needs to know about the issue or pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub is_pull_request: bool,
    pub author: String,
    pub author_is_bot: bool,
    pub assignees: Vec<String>,
}

/// Knobs for [`plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanOptions {
    /// Add owners even when someone is already assigned.
    pub force: bool,
    /// Leave issues and pull requests opened by bots (Renovate, Dependabot)
    /// unassigned.
    pub skip_bots: bool,
}

/// The decision for one issue or pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Skip(String),
    Assign(Vec<String>),
}

/// Decides who to add, given the candidate logins. Never plans more than
/// [`MAX_ASSIGNEES`] in total, and never re-adds someone already assigned.
pub fn plan(issue: &Issue, candidates: &[String], options: &PlanOptions) -> Plan {
    if options.skip_bots && issue.author_is_bot {
        return Plan::Skip(format!(
            "#{} was opened by bot {}",
            issue.number, issue.author
        ));
    }
    if !options.force && !issue.assignees.is_empty() {
        return Plan::Skip(format!(
            "#{} is already assigned to {}",
            issue.number,
            issue.assignees.join(", ")
        ));
    }

    let room = MAX_ASSIGNEES.saturating_sub(issue.assignees.len());
    let to_add: Vec<String> = candidates
        .iter()
        .filter(|login| {
            !issue
                .assignees
                .iter()
                .any(|assigned| assigned.eq_ignore_ascii_case(login))
        })
        .take(room)
        .cloned()
        .collect();

    if to_add.is_empty() {
        Plan::Skip(format!("no one new to assign to #{}", issue.number))
    } else {
        Plan::Assign(to_add)
    }
}

/// The GitHub REST calls `paws assign` makes.
pub struct GitHubAssignClient {
    owner: String,
    repo: String,
    token: String,
    client: reqwest::Client,
}

/// Hand-written, not derived: `token` is a live GitHub credential.
impl std::fmt::Debug for GitHubAssignClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAssignClient")
            .field("owner", &self.owner)
            .field("repo", &self.repo)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl GitHubAssignClient {
    pub fn new(owner: String, repo: String, token: String) -> Self {
        Self {
            owner,
            repo,
            token,
            client: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/{path}",
            self.owner, self.repo
        )
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, self.url(path))
            .bearer_auth(&self.token)
            .header("User-Agent", "paws-assign")
            .header("Accept", "application/vnd.github+json")
    }

    async fn json(builder: reqwest::RequestBuilder, what: &str) -> Result<serde_json::Value> {
        let response = builder
            .send()
            .await
            .with_context(|| format!("failed to reach GitHub for {what}"))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("{what} failed: {status}: {text}");
        }
        response
            .json()
            .await
            .with_context(|| format!("failed to parse GitHub's response for {what}"))
    }

    /// The `CODEOWNERS` file on the default branch, as `(path, contents)`.
    /// Read from the API rather than a checkout so a `pull_request_target`
    /// job never needs to check out (or trust) the pull request's own copy.
    pub async fn codeowners(&self) -> Result<Option<(String, String)>> {
        for path in CODEOWNERS_PATHS {
            let response = self
                .request(reqwest::Method::GET, &format!("contents/{path}"))
                .header("Accept", "application/vnd.github.raw+json")
                .send()
                .await
                .context("failed to reach GitHub for CODEOWNERS")?;
            let status = response.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                continue;
            }
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                bail!("reading {path} failed: {status}: {text}");
            }
            let text = response
                .text()
                .await
                .with_context(|| format!("failed to read {path}"))?;
            return Ok(Some(((*path).to_string(), text)));
        }
        Ok(None)
    }

    pub async fn issue(&self, number: u64) -> Result<Issue> {
        let body = Self::json(
            self.request(reqwest::Method::GET, &format!("issues/{number}")),
            &format!("reading #{number}"),
        )
        .await?;
        Ok(issue_from_response(number, &body))
    }

    /// Every path a pull request touches, including the old side of renames
    /// (moving a file out of an owned directory concerns its owners too).
    pub async fn pull_request_files(&self, number: u64) -> Result<Vec<String>> {
        const PER_PAGE: usize = 100;
        let mut paths = Vec::new();
        // GitHub stops listing at 3000 files.
        for page in 1..=30 {
            let body = Self::json(
                self.request(
                    reqwest::Method::GET,
                    &format!("pulls/{number}/files?per_page={PER_PAGE}&page={page}"),
                ),
                &format!("listing the files of #{number}"),
            )
            .await?;
            let files = body.as_array().map(Vec::as_slice).unwrap_or_default();
            for file in files {
                for key in ["filename", "previous_filename"] {
                    if let Some(path) = file.get(key).and_then(serde_json::Value::as_str) {
                        paths.push(path.to_string());
                    }
                }
            }
            if files.len() < PER_PAGE {
                break;
            }
        }
        Ok(paths)
    }

    /// Adds `logins` and returns everyone assigned afterwards. GitHub drops
    /// logins that can't be assigned (no access to the repo) without an
    /// error, so callers compare the result against what they asked for.
    pub async fn add_assignees(&self, number: u64, logins: &[String]) -> Result<Vec<String>> {
        let body = Self::json(
            self.request(reqwest::Method::POST, &format!("issues/{number}/assignees"))
                .json(&serde_json::json!({ "assignees": logins })),
            &format!("assigning #{number}"),
        )
        .await?;
        Ok(issue_from_response(number, &body).assignees)
    }
}

fn issue_from_response(number: u64, body: &serde_json::Value) -> Issue {
    let login = |value: &serde_json::Value| {
        value
            .get("login")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    Issue {
        number,
        is_pull_request: body.get("pull_request").is_some(),
        author: body.get("user").and_then(login).unwrap_or_default(),
        author_is_bot: body
            .pointer("/user/type")
            .and_then(serde_json::Value::as_str)
            == Some("Bot"),
        assignees: body
            .get("assignees")
            .and_then(serde_json::Value::as_array)
            .map(|assignees| assignees.iter().filter_map(login).collect())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn issue(assignees: &[&str]) -> Issue {
        Issue {
            number: 7,
            is_pull_request: false,
            author: "someone".into(),
            author_is_bot: false,
            assignees: strings(assignees),
        }
    }

    #[test]
    fn parse_codeowners_skips_comments_and_strips_trailing_ones() {
        let rules =
            parse_codeowners("# owners\n\n* @alice @org/team\n/docs/ @bob # docs folks\n*.lock\n");
        assert_eq!(
            rules,
            vec![
                Rule {
                    pattern: "*".into(),
                    owners: strings(&["@alice", "@org/team"]),
                },
                Rule {
                    pattern: "/docs/".into(),
                    owners: strings(&["@bob"]),
                },
                Rule {
                    pattern: "*.lock".into(),
                    owners: vec![],
                },
            ]
        );
    }

    #[test]
    fn unanchored_patterns_match_at_any_depth() {
        assert!(pattern_matches("*", "README.md"));
        assert!(pattern_matches("*", "src/deep/main.rs"));
        assert!(pattern_matches("*.rs", "crates/a/src/lib.rs"));
        assert!(!pattern_matches("*.rs", "crates/a/Cargo.toml"));
        assert!(pattern_matches("apps/", "nested/apps/web/index.ts"));
        assert!(pattern_matches("Makefile", "sub/Makefile"));
    }

    #[test]
    fn a_slash_anchors_the_pattern_to_the_root() {
        assert!(pattern_matches("/docs/", "docs/guide.md"));
        assert!(!pattern_matches("/docs/", "src/docs/guide.md"));
        assert!(pattern_matches("docs/*.md", "docs/guide.md"));
        assert!(!pattern_matches("docs/*.md", "docs/deep/guide.md"));
        assert!(pattern_matches("/build/logs", "build/logs/today.log"));
        assert!(!pattern_matches("/Makefile", "sub/Makefile"));
    }

    #[test]
    fn trailing_slash_only_matches_directories() {
        assert!(!pattern_matches("docs/", "docs"));
        assert!(pattern_matches("docs", "docs"));
    }

    #[test]
    fn double_star_spans_segments() {
        assert!(pattern_matches("**/logs", "a/b/logs/x.log"));
        assert!(pattern_matches("docs/**/*.md", "docs/a/b/c.md"));
        assert!(pattern_matches("docs/**/*.md", "docs/c.md"));
        assert!(!pattern_matches("docs/**/*.md", "src/c.md"));
    }

    #[test]
    fn question_mark_and_escapes() {
        assert!(pattern_matches("file?.txt", "file1.txt"));
        assert!(!pattern_matches("file?.txt", "file10.txt"));
        assert!(pattern_matches("\\*.txt", "*.txt"));
        assert!(!pattern_matches("\\*.txt", "a.txt"));
    }

    #[test]
    fn the_last_matching_rule_wins_per_path() {
        let rules = parse_codeowners("* @alice\n/docs/ @bob\n*.lock\n");
        assert_eq!(
            owners_for_paths(&rules, &["docs/a.md", "src/main.rs", "docs/b.md"]),
            strings(&["@bob", "@alice"])
        );
        assert_eq!(
            owners_for_paths(&rules, &["Cargo.lock"]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn default_owners_come_from_the_last_catch_all_rule() {
        let rules = parse_codeowners("* @alice\n/docs/ @bob\n** @carol\n");
        assert_eq!(default_owners(&rules), strings(&["@carol"]));
        assert!(default_owners(&parse_codeowners("/docs/ @bob\n")).is_empty());
    }

    #[test]
    fn teams_and_emails_are_not_assignable() {
        let (logins, skipped) =
            assignable_logins(&strings(&["@alice", "@org/team", "a@b.dev", "@alice"]));
        assert_eq!(logins, strings(&["alice"]));
        assert_eq!(skipped, strings(&["@org/team", "a@b.dev"]));
    }

    #[test]
    fn number_from_event_reads_issue_and_pull_request_payloads() {
        let issue_event = serde_json::json!({ "issue": { "number": 12 } });
        let pr_event = serde_json::json!({ "number": 5, "pull_request": { "number": 5 } });
        assert_eq!(number_from_event(&issue_event), Some(12));
        assert_eq!(number_from_event(&pr_event), Some(5));
        assert_eq!(number_from_event(&serde_json::json!({})), None);
    }

    #[test]
    fn plan_leaves_already_assigned_issues_alone_unless_forced() {
        let candidates = strings(&["alice"]);
        assert!(matches!(
            plan(&issue(&["bob"]), &candidates, &PlanOptions::default()),
            Plan::Skip(_)
        ));
        let forced = PlanOptions {
            force: true,
            ..PlanOptions::default()
        };
        assert_eq!(
            plan(&issue(&["bob"]), &candidates, &forced),
            Plan::Assign(candidates)
        );
    }

    #[test]
    fn plan_skips_bots_only_when_asked() {
        let bot = Issue {
            author: "renovate[bot]".into(),
            author_is_bot: true,
            ..issue(&[])
        };
        let candidates = strings(&["alice"]);
        assert_eq!(
            plan(&bot, &candidates, &PlanOptions::default()),
            Plan::Assign(candidates.clone())
        );
        let skip_bots = PlanOptions {
            skip_bots: true,
            ..PlanOptions::default()
        };
        assert!(matches!(plan(&bot, &candidates, &skip_bots), Plan::Skip(_)));
    }

    #[test]
    fn plan_never_readds_or_exceeds_the_assignee_limit() {
        let forced = PlanOptions {
            force: true,
            ..PlanOptions::default()
        };
        let current = Issue {
            assignees: (0..9).map(|i| format!("user{i}")).collect(),
            ..issue(&[])
        };
        let candidates = strings(&["USER0", "alice", "bob"]);
        assert_eq!(
            plan(&current, &candidates, &forced),
            Plan::Assign(strings(&["alice"]))
        );
        assert!(matches!(plan(&issue(&[]), &[], &forced), Plan::Skip(_)));
    }

    #[test]
    fn issue_from_response_reads_author_assignees_and_kind() {
        let body = serde_json::json!({
            "user": { "login": "renovate[bot]", "type": "Bot" },
            "assignees": [{ "login": "alice" }],
            "pull_request": { "url": "..." },
        });
        assert_eq!(
            issue_from_response(3, &body),
            Issue {
                number: 3,
                is_pull_request: true,
                author: "renovate[bot]".into(),
                author_is_bot: true,
                assignees: strings(&["alice"]),
            }
        );
    }
}
