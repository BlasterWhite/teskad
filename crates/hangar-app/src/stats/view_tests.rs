//! The page's agent parts: when the "by agent" card shows, what is said under
//! each agent, and how the model breakdown names models per kind. Pure: the
//! figures come from `summarize_by_agent` over statistics written here, so
//! no window is needed.

use hangar_core::stats::{Bucket, ModelInfo, ScanInfo, SessionInfo, UsageStats, UsageStatsByAgent};
use hangar_i18n::{Locale, Translator};

use super::format::format_usd;
use super::model::{Measure, Period, Project, Ranked, Summary, SummaryOptions, TOP_SESSIONS};
use super::time::testing::at;
use super::view::{agent_details, both_agents, models_by_agent};
use crate::state::types::AgentKind;

const EN: Locale = Locale::En;
const FR: Locale = Locale::Fr;

/// Tokens of the only kind a row below carries, in the `output` column.
fn row(session: u32, model: i32, tokens: u64, cost: f64) -> Bucket {
    Bucket {
        session,
        at: at(25, 10),
        model,
        output: tokens,
        cost,
        calls: 1,
        ..Bucket::default()
    }
}

/// Active time sits in a row of its own, with no model, as hangar-core writes
/// it.
fn active(session: u32, ms: i64) -> Bucket {
    Bucket {
        session,
        at: at(25, 10),
        model: -1,
        active_ms: ms,
        turns: 1,
        ..Bucket::default()
    }
}

fn session(id: &str) -> SessionInfo {
    SessionInfo {
        id: id.into(),
        cwd: "/home/me/app".into(),
        title: id.into(),
        first_at: at(25, 10),
        last_at: at(25, 10),
    }
}

/// A Claude Code session and two Codex ones (one on a model with no price).
/// Codex has the most tokens.
fn both() -> UsageStatsByAgent {
    UsageStatsByAgent {
        stats: UsageStats {
            generated_at: at(26, 15),
            scan: ScanInfo::default(),
            sources: Vec::new(),
            active_gap_cap_ms: 300_000,
            sessions: vec![session("a"), session("b"), session("c")],
            models: vec![
                ModelInfo {
                    id: "claude-opus-5-5".into(),
                    priced: true,
                },
                ModelInfo {
                    id: "gpt-5.3-codex".into(),
                    priced: true,
                },
                ModelInfo {
                    id: "gpt-5.5-codex".into(),
                    priced: false,
                },
            ],
            buckets: vec![
                row(0, 0, 1_000, 3.0),
                active(0, 11_100_000),
                row(1, 1, 4_000, 2.0),
                active(1, 1_800_000),
                row(2, 2, 500, 0.0),
                active(2, 600_000),
            ],
        },
        session_agents: vec![AgentKind::Claude, AgentKind::Codex, AgentKind::Codex],
    }
}

/// The same page of a person who never ran Codex.
fn claude_only() -> UsageStatsByAgent {
    let mut data = both();
    data.stats.buckets.retain(|bucket| bucket.session == 0);
    data
}

fn summary(data: &UsageStatsByAgent, measure: Measure) -> Summary {
    let projects = vec![Project::default(); data.stats.sessions.len()];
    super::model::summarize_by_agent(
        data,
        &SummaryOptions {
            period: Period::D7,
            project: None,
            measure,
            now: at(26, 15),
            projects: &projects,
            top_sessions: TOP_SESSIONS,
        },
    )
}

fn agent<'s>(summary: &'s Summary, id: &str) -> &'s Ranked {
    summary
        .agents
        .iter()
        .find(|row| row.key == id)
        .unwrap_or_else(|| panic!("no {id} row in {:?}", summary.agents))
}

fn labels(rows: &[Ranked]) -> Vec<&str> {
    rows.iter().map(|row| row.label.as_str()).collect()
}

#[test]
fn the_card_shows_once_both_agents_worked_and_not_for_one() {
    assert!(both_agents(&summary(&both(), Measure::Tokens)));
    let alone = summary(&claude_only(), Measure::Tokens);
    assert!(!both_agents(&alone));
    // Nothing at all: no card either.
    let mut empty = both();
    empty.stats.buckets.clear();
    assert!(!both_agents(&summary(&empty, Measure::Tokens)));
}

#[test]
fn a_codex_only_history_has_no_card_either() {
    let mut data = both();
    data.stats.buckets.retain(|bucket| bucket.session != 0);
    let summary = summary(&data, Measure::Tokens);
    assert!(!both_agents(&summary));
    assert_eq!(summary.agents.len(), 1);
    assert_eq!(summary.agents[0].key, "codex");
}

#[test]
fn the_agents_rank_by_the_measure_with_their_own_totals() {
    let by_tokens = summary(&both(), Measure::Tokens);
    let keys: Vec<&str> = by_tokens
        .agents
        .iter()
        .map(|row| row.key.as_str())
        .collect();
    assert_eq!(keys, ["codex", "claude"]);
    assert_eq!(by_tokens.agents[0].label, "Codex");
    assert_eq!(agent(&by_tokens, "codex").totals.tokens, 4_500);
    assert_eq!(agent(&by_tokens, "codex").totals.sessions, 2);
    assert_eq!(agent(&by_tokens, "claude").totals.tokens, 1_000);
    assert_eq!(agent(&by_tokens, "claude").totals.active_ms, 11_100_000);

    // By cost, Claude Code's 3 dollars come first.
    let by_cost = summary(&both(), Measure::Cost);
    assert_eq!(by_cost.agents[0].key, "claude");
}

#[test]
fn what_is_said_under_an_agent_is_the_figure_the_ranking_leaves_out_and_its_active_time() {
    let en = Translator::new(EN);
    let fr = Translator::new(FR);
    let tokens = summary(&both(), Measure::Tokens);
    // Ranked by tokens: the cost, then the time (3 h 05).
    assert_eq!(
        agent_details(en, agent(&tokens, "claude"), Measure::Tokens),
        format!("{} · 3\u{a0}h\u{a0}05 active", format_usd(3.0, EN, false))
    );
    assert_eq!(
        agent_details(fr, agent(&tokens, "claude"), Measure::Tokens),
        format!(
            "{} · 3\u{a0}h\u{a0}05\u{a0}d'activité",
            format_usd(3.0, FR, false)
        )
    );
    // Ranked by cost: the tokens.
    let cost = summary(&both(), Measure::Cost);
    assert_eq!(
        agent_details(en, agent(&cost, "claude"), Measure::Cost),
        "1K tokens · 3\u{a0}h\u{a0}05 active"
    );
    assert_eq!(
        agent_details(en, agent(&cost, "codex"), Measure::Cost),
        "4.5K tokens · 40\u{a0}min active"
    );
}

#[test]
fn an_agent_with_nothing_priced_says_so_rather_than_show_no_cost() {
    let en = Translator::new(EN);
    let mut data = both();
    // Codex ran only the model with no price.
    data.stats.buckets.retain(|bucket| bucket.session != 1);
    let summary = summary(&data, Measure::Tokens);
    let codex = agent(&summary, "codex");
    assert!(codex.unpriced);
    assert_eq!(
        agent_details(en, codex, Measure::Tokens),
        "No known price · 10\u{a0}min active"
    );
    // A kind with some price keeps its cost: it is what the priced part cost.
    let mixed = self::summary(&both(), Measure::Tokens);
    assert!(agent(&mixed, "codex").unpriced);
    assert!(agent_details(en, agent(&mixed, "codex"), Measure::Tokens).starts_with("$2"));
}

#[test]
fn the_models_fall_under_their_agent_in_the_order_the_agents_rank() {
    let summary = summary(&both(), Measure::Tokens);
    let groups = models_by_agent(&summary);
    let kinds: Vec<AgentKind> = groups.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(kinds, [AgentKind::Codex, AgentKind::Claude]);
    assert_eq!(labels(&groups[0].1), ["GPT-5.3 Codex", "GPT-5.5 Codex"]);
    assert_eq!(labels(&groups[1].1), ["Opus 5.5"]);
    // The id behind a name is still the row's detail.
    assert_eq!(groups[0].1[0].detail, "gpt-5.3-codex");
    // Grouping loses no model.
    let grouped: usize = groups.iter().map(|(_, rows)| rows.len()).sum();
    assert_eq!(grouped, summary.models.len());
}

#[test]
fn by_cost_the_group_of_the_heaviest_agent_comes_first() {
    let summary = summary(&both(), Measure::Cost);
    let kinds: Vec<AgentKind> = models_by_agent(&summary)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    assert_eq!(kinds, [AgentKind::Claude, AgentKind::Codex]);
}

#[test]
fn a_claude_only_page_keeps_its_one_flat_model_list() {
    let summary = summary(&claude_only(), Measure::Tokens);
    assert!(!both_agents(&summary));
    assert_eq!(labels(&summary.models), ["Opus 5.5"]);
    assert_eq!(summary.agents.len(), 1);
    assert_eq!(summary.agents[0].key, "claude");
    // Its one agent's figures are the page's totals.
    assert_eq!(summary.agents[0].totals, summary.totals);
}
