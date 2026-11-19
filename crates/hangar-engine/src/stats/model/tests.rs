//! The cases of `lib/statsModel.test.ts`, with the model-name and path cases
//! the page relies on added. Local times throughout, so the tests hold in any
//! time zone.

use hangar_core::stats::{Bucket, ModelInfo, ScanInfo, SessionInfo, UsageStats, UsageStatsByAgent};

use super::super::time::testing::{at, on};
use super::*;
use crate::state::types::Pane;

const NOW_DAY: u32 = 26;

fn now() -> i64 {
    at(NOW_DAY, 15)
}

fn workspace(id: &str, cwd: &str, extra: &[&str]) -> Workspace {
    Workspace {
        id: id.to_owned(),
        name: id.to_owned(),
        cwd: cwd.to_owned(),
        extra_roots: extra.iter().map(|root| (*root).to_owned()).collect(),
        ..Workspace::default()
    }
}

fn pane(cwd: &str) -> Pane {
    Pane {
        id: cwd.to_owned(),
        cwd: Some(cwd.to_owned()),
        ..Pane::default()
    }
}

/// A bucket as the TS tests' `bucket()` makes it: 1000 tokens, 2 calls.
fn bucket(session: u32, when: i64, patch: impl FnOnce(&mut Bucket)) -> Bucket {
    let mut row = Bucket {
        session,
        at: when,
        model: 0,
        input: 10,
        output: 20,
        cache_write: 30,
        cache_read: 940,
        cost: 1.0,
        cost_input: 0.1,
        cost_output: 0.5,
        cost_cache_write: 0.2,
        cost_cache_read: 0.2,
        calls: 2,
        turns: 0,
        active_ms: 0,
    };
    patch(&mut row);
    row
}

fn plain(session: u32, when: i64) -> Bucket {
    bucket(session, when, |_| {})
}

fn stats(buckets: Vec<Bucket>) -> UsageStats {
    UsageStats {
        generated_at: now(),
        scan: ScanInfo {
            files: 2,
            parsed: 2,
            ms: 1,
            ..ScanInfo::default()
        },
        sources: Vec::new(),
        active_gap_cap_ms: 300_000,
        sessions: vec![
            SessionInfo {
                id: "a".into(),
                cwd: "C:\\dev\\app".into(),
                title: "Auth".into(),
                first_at: at(20, 10),
                last_at: at(25, 10),
            },
            SessionInfo {
                id: "b".into(),
                cwd: "C:\\dev\\shop".into(),
                title: String::new(),
                first_at: at(24, 10),
                last_at: at(24, 10),
            },
        ],
        models: vec![
            ModelInfo {
                id: "claude-opus-5-5".into(),
                priced: true,
            },
            ModelInfo {
                id: "house-model".into(),
                priced: false,
            },
        ],
        buckets,
    }
}

fn projects() -> Vec<Project> {
    vec![
        Project {
            key: "ws:app".into(),
            name: "app".into(),
            path: "c:/dev/app".into(),
            workspace_id: Some("app".into()),
        },
        Project {
            key: "dir:c:/dev/shop".into(),
            name: "shop".into(),
            path: "C:\\dev\\shop".into(),
            workspace_id: None,
        },
    ]
}

fn summary_of(
    data: &UsageStats,
    period: Period,
    project: Option<&str>,
    measure: Measure,
) -> Summary {
    let projects = projects();
    summarize(
        data,
        &SummaryOptions {
            period,
            project,
            measure,
            now: now(),
            projects: &projects,
            top_sessions: TOP_SESSIONS,
        },
    )
}

#[test]
fn a_period_covers_whole_days_up_to_today_and_the_same_length_before_for_the_trend() {
    let range = period_range(Period::D7, now(), None);
    assert_eq!(range.to, start_of_day(at(27, 12)));
    assert_eq!(range.from, start_of_day(at(20, 12)));
    assert_eq!(range.days, 7);
    assert_eq!(
        range.previous,
        Some((start_of_day(at(13, 12)), start_of_day(at(20, 12))))
    );
}

#[test]
fn all_starts_on_the_first_day_with_data_with_nothing_to_compare() {
    let range = period_range(Period::All, now(), Some(at(3, 22)));
    assert_eq!(range.from, start_of_day(at(3, 12)));
    assert_eq!(range.days, 24);
    assert_eq!(range.previous, None);
}

#[test]
fn all_with_no_data_is_today() {
    let range = period_range(Period::All, now(), None);
    assert_eq!(range.days, 1);
    assert_eq!(range.from, start_of_day(now()));
}

#[test]
fn spells_a_windows_path_one_way_case_and_slashes_aside() {
    assert_eq!(normalize_path("C:\\Dev\\App\\"), "c:/dev/app");
    assert_eq!(normalize_path("/home/Me/app/"), "/home/Me/app");
    assert_eq!(normalize_path("C:\\"), "c:/");
    assert_eq!(normalize_path("  /srv/x//  "), "/srv/x");
    assert_eq!(normalize_path("//Server/Share/"), "//server/share");
    assert_eq!(normalize_path("/"), "/");
}

#[test]
fn finds_the_workspace_with_the_deepest_root_above_a_folder() {
    let worktrees: HashMap<String, Vec<String>> =
        HashMap::from([("app".to_owned(), vec!["C:\\dev\\app-stats".to_owned()])]);
    let roots = workspace_roots(
        &[
            workspace("mono", "C:\\dev\\mono", &[]),
            workspace("web", "C:\\dev\\mono\\web", &[]),
            workspace("app", "C:\\dev\\app", &["D:\\shared\\lib"]),
        ],
        &worktrees,
    );
    assert_eq!(project_for("c:/DEV/mono/web/src", &roots).key, "ws:web");
    assert_eq!(project_for("C:\\dev\\mono\\api", &roots).key, "ws:mono");
    assert_eq!(project_for("C:\\dev\\app-stats", &roots).key, "ws:app");
    assert_eq!(project_for("D:\\shared\\lib\\x", &roots).name, "app");
    // A sibling whose name merely starts the same is not inside.
    assert_eq!(
        project_for("C:\\dev\\application", &roots).key,
        "dir:c:/dev/application"
    );
    assert_eq!(
        project_for("C:\\dev\\application", &roots).name,
        "application"
    );
}

#[test]
fn counts_a_panes_folder_unless_it_sits_above_the_workspace() {
    let mut app = workspace("app", "C:\\Users\\me\\dev\\app", &[]);
    app.panes = vec![pane("C:\\Users\\me"), pane("C:\\Users\\me\\dev\\app-wt")];
    let roots = workspace_roots(&[app], &HashMap::new());
    assert_eq!(
        project_for("C:\\Users\\me\\dev\\app-wt", &roots).key,
        "ws:app"
    );
    assert_eq!(
        project_for("C:\\Users\\me\\other", &roots).key,
        "dir:c:/users/me/other"
    );
}

#[test]
fn a_folder_listed_twice_is_one_root() {
    let mut app = workspace("app", "/home/me/app", &["/home/me/app/"]);
    app.panes = vec![pane("/home/me/app")];
    let roots = workspace_roots(&[app], &HashMap::new());
    assert_eq!(roots.len(), 1);
}

fn summary_rows() -> Vec<Bucket> {
    vec![
        bucket(0, at(22, 9), |row| {
            row.active_ms = 600_000;
            row.turns = 2;
        }),
        bucket(0, at(25, 14), |row| {
            row.output = 1000;
            row.cost = 3.0;
            row.cost_output = 2.5;
        }),
        bucket(1, at(24, 9), |row| {
            row.model = 1;
            row.cost = 0.0;
            row.cost_input = 0.0;
            row.cost_output = 0.0;
            row.cost_cache_write = 0.0;
            row.cost_cache_read = 0.0;
        }),
        // The week before: the trend's material.
        plain(0, at(15, 11)),
        // Out of every period but "all".
        plain(0, at(2, 11)),
    ]
}

#[test]
fn sums_the_period_fills_the_quiet_days_and_compares_with_the_one_before() {
    let summary = summary_of(&stats(summary_rows()), Period::D7, None, Measure::Tokens);
    assert_eq!(summary.series.len(), 7);
    let tokens: Vec<u64> = summary
        .series
        .iter()
        .map(|point| point.totals.tokens)
        .collect();
    assert_eq!(tokens, [0, 0, 1000, 0, 1000, 1980, 0]);
    assert_eq!(summary.totals.tokens, 3980);
    assert_eq!(summary.totals.sessions, 2);
    assert_eq!(summary.totals.turns, 2);
    let previous = summary.previous.as_ref().unwrap();
    assert_eq!(previous.tokens, 1000);
    assert_eq!(previous.sessions, 1);
    assert_eq!(summary.active_days, 1);
    // Thursday 22 October, 9 h.
    assert_eq!(summary.heat[3][9], 600_000);
    assert_eq!(summary.heat_max, 600_000);
}

#[test]
fn ranks_by_the_measure_and_keeps_every_project_pickable_under_a_filter() {
    let data = stats(summary_rows());
    let filtered = summary_of(&data, Period::D7, Some("ws:app"), Measure::Cost);
    assert_eq!(filtered.totals.sessions, 1);
    let keys: Vec<&str> = filtered
        .projects
        .iter()
        .map(|row| row.key.as_str())
        .collect();
    assert_eq!(keys, ["ws:app", "dir:c:/dev/shop"]);
    assert_eq!(filtered.projects[0].share, 1.0);
    let agents: Vec<&str> = filtered
        .agents
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(agents, ["Claude Code"]);
    assert_eq!(filtered.agents[0].key, "claude");
    assert_eq!(filtered.agents[0].totals.sessions, 1);
    assert_eq!(filtered.sessions[0].model.as_deref(), Some("Opus 5.5"));
}

#[test]
fn puts_the_tokens_of_unpriced_models_aside_and_names_them() {
    let summary = summary_of(&stats(summary_rows()), Period::D7, None, Measure::Tokens);
    assert_eq!(summary.unpriced_tokens, 1000);
    let odd = summary
        .models
        .iter()
        .find(|row| row.key == "house-model")
        .unwrap();
    assert!(odd.unpriced);
    assert_eq!(odd.label, "house-model");
    assert_eq!(odd.detail, "");
    let opus = summary
        .models
        .iter()
        .find(|row| row.key == "claude-opus-5-5")
        .unwrap();
    assert_eq!(opus.detail, "claude-opus-5-5");
}

#[test]
fn orders_the_heaviest_sessions_by_the_measure() {
    let data = stats(summary_rows());
    let by_tokens = summary_of(&data, Period::D7, None, Measure::Tokens).sessions;
    let ids: Vec<&str> = by_tokens.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    let by_cost = summary_of(&data, Period::D7, None, Measure::Cost).sessions;
    assert_eq!(by_cost[0].id, "a");
    assert_eq!(by_cost[0].totals.cost, 4.0);
}

#[test]
fn counts_weeks_rather_than_days_past_120_days() {
    let old = vec![plain(0, add_days(at(26, 10), -200)), plain(0, at(26, 10))];
    let summary = summary_of(&stats(old), Period::All, None, Measure::Tokens);
    assert_eq!(summary.granularity, Granularity::Week);
    assert!(summary.series.len() > 28, "{} weeks", summary.series.len());
    assert!(summary.series.len() < 32, "{} weeks", summary.series.len());
    // Every column starts on a Monday.
    assert_eq!(weekday_of(summary.series[0].at), 0);
    assert_eq!(summary.totals.tokens, 2000);
}

#[test]
fn a_session_without_a_project_row_keeps_its_own_folder() {
    let data = stats(vec![plain(0, at(25, 10))]);
    let none: [Project; 0] = [];
    let summary = summarize(
        &data,
        &SummaryOptions {
            period: Period::D7,
            project: None,
            measure: Measure::Tokens,
            now: now(),
            projects: &none,
            top_sessions: TOP_SESSIONS,
        },
    );
    assert!(summary.projects.is_empty());
    assert_eq!(summary.sessions[0].project.path, "C:\\dev\\app");
}

#[test]
fn names_models_the_way_people_say_them() {
    assert_eq!(model_label("claude-opus-5-5"), "Opus 5.5");
    assert_eq!(model_label("gpt-5.6-sol"), "GPT-5.6 Sol");
    assert_eq!(model_label("gpt-5.5"), "GPT-5.5");
    assert_eq!(model_label("house-model"), "house-model");
}

#[test]
fn names_openai_models_by_their_line() {
    for (id, label) in [
        ("gpt-5.3-codex", "GPT-5.3 Codex"),
        ("gpt-5.1-codex-max", "GPT-5.1 Codex Max"),
        ("gpt-5.4-mini", "GPT-5.4 Mini"),
        ("gpt-5.5-pro", "GPT-5.5 Pro"),
        ("GPT-6.2-Sol", "GPT-6.2 Sol"),
        ("  gpt-5.5  ", "GPT-5.5"),
        // A dated snapshot is the same model.
        ("gpt-5-2025-08-07", "GPT-5"),
        ("gpt-5.4-mini-2026-03-17", "GPT-5.4 Mini"),
        ("codex-mini-latest", "Codex Mini"),
        // What is not of a family Hangar reads comes back as written.
        ("o3", "o3"),
        ("gpt-4o", "gpt-4o"),
        ("gpt-oss", "gpt-oss"),
        ("gpt-5-", "gpt-5-"),
        ("codex-", "codex-"),
        ("codex-latest", "codex-latest"),
    ] {
        assert_eq!(model_label(id), label, "{id}");
    }
    // The Claude names are the ones they were.
    assert_eq!(model_label("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
}

#[test]
fn tells_the_agent_of_a_model_by_its_makers_naming() {
    for id in [
        "gpt-5.5",
        "GPT-6.2-Sol",
        "gpt-5.3-codex",
        "codex-mini-latest",
        "o3",
        "o4-mini",
    ] {
        assert_eq!(model_agent(id), AgentKind::Codex, "{id}");
    }
    for id in [
        "claude-opus-5-5",
        "claude-opus-5-5[1m]",
        "opus",
        "house-model",
        "omni-x",
        "o3x",
        "",
    ] {
        assert_eq!(model_agent(id), AgentKind::default(), "{id}");
    }
}

#[test]
fn pretty_model_reads_every_claude_naming() {
    for (id, label) in [
        ("claude-sonnet-4-5-20250929", "Sonnet 4.5"),
        ("claude-opus-4-20250514", "Opus 4"),
        ("claude-opus-4-1", "Opus 4.1"),
        ("claude-opus-4.1", "Opus 4.1"),
        ("claude-haiku-4-5", "Haiku 4.5"),
        ("claude-sonnet-4-5[1m]", "Sonnet 4.5"),
        ("us.anthropic.claude-opus-4-5", "Opus 4.5"),
        ("claude-3-5-sonnet-20241022", "Sonnet 3.5"),
        ("claude-3-opus-20240229", "Opus 3"),
        ("opus", "Opus"),
        ("sonnet-4-6", "Sonnet 4.6"),
        ("claude-fable-1", "Fable 1"),
        ("  something-else  ", "something-else"),
        ("claude-opus-x", "claude-opus-x"),
    ] {
        assert_eq!(pretty_model(id), label, "{id}");
    }
}

#[test]
fn steps_the_heatmap_by_the_square_root_of_the_value() {
    assert_eq!(heat_level(0, 100), 0);
    assert_eq!(heat_level(1, 100), 1);
    assert_eq!(heat_level(25, 100), 3);
    assert_eq!(heat_level(100, 100), 5);
    assert_eq!(heat_level(50, 0), 0);
}

#[test]
fn a_trend_needs_something_to_compare_with() {
    assert_eq!(change(5.0, 0.0), None);
    assert_eq!(change(150.0, 100.0), Some(0.5));
    assert_eq!(change(50.0, 100.0), Some(-0.5));
}

#[test]
fn the_period_ids_round_trip() {
    for period in PERIODS {
        assert_eq!(Period::from_id(period.id()), Some(period));
    }
    assert_eq!(Period::from_id("1y"), None);
    assert_eq!(on(10, 1, 0), start_of_day(on(10, 1, 13)));
}

// ---------------------------------------------------------------------------
// By agent
// ---------------------------------------------------------------------------

/// Three sessions in the period, a Claude Code one (`a`, app) and two Codex
/// ones (`b`, shop; `c`, app), the second with a model the price table does
/// not know.
fn mixed() -> UsageStatsByAgent {
    let mut data = stats(vec![
        plain(0, at(22, 9)),
        bucket(0, at(25, 14), |row| {
            row.output = 1000;
            row.cost = 3.0;
        }),
        bucket(1, at(24, 9), |row| {
            row.model = 2;
            row.cost = 2.0;
        }),
        bucket(2, at(25, 10), |row| {
            row.model = 3;
            row.cost = 0.0;
        }),
        // The week before: the trend's material.
        plain(0, at(15, 11)),
    ]);
    data.sessions.push(SessionInfo {
        id: "c".into(),
        cwd: "C:\\dev\\app".into(),
        title: "Review".into(),
        first_at: at(25, 10),
        last_at: at(25, 10),
    });
    data.models.push(ModelInfo {
        id: "gpt-5.3-codex".into(),
        priced: true,
    });
    data.models.push(ModelInfo {
        id: "gpt-5.5-codex".into(),
        priced: false,
    });
    UsageStatsByAgent {
        stats: data,
        session_agents: vec![AgentKind::default(), AgentKind::Codex, AgentKind::Codex],
    }
}

fn by_agent(data: &UsageStatsByAgent, project: Option<&str>, measure: Measure) -> Summary {
    let mut projects = projects();
    projects.push(projects[0].clone());
    summarize_by_agent(
        data,
        &SummaryOptions {
            period: Period::D7,
            project,
            measure,
            now: now(),
            projects: &projects,
            top_sessions: TOP_SESSIONS,
        },
    )
}

fn agent_row<'s>(summary: &'s Summary, key: &str) -> &'s Ranked {
    summary
        .agents
        .iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("no {key} row in {:?}", summary.agents))
}

#[test]
fn a_claude_only_history_is_one_claude_code_row_and_the_totals_do_not_move() {
    let data = stats(summary_rows());
    let by_model = summary_of(&data, Period::D7, None, Measure::Tokens);
    let exact = by_agent(
        &UsageStatsByAgent {
            stats: data.clone(),
            session_agents: vec![AgentKind::default(); data.sessions.len()],
        },
        None,
        Measure::Tokens,
    );
    for summary in [&by_model, &exact] {
        assert_eq!(summary.agents.len(), 1);
        let row = &summary.agents[0];
        assert_eq!(
            (row.key.as_str(), row.label.as_str()),
            ("claude", "Claude Code")
        );
        assert_eq!(row.share, 1.0);
        assert_eq!(row.totals, summary.totals);
        // The house model has no price, and its row says so.
        assert!(row.unpriced);
    }
    assert_eq!(by_model.totals.tokens, 1000 + 1980 + 1000);
    // Everything but the agents' rows is the same whichever way it is read.
    assert_eq!(by_model.totals, exact.totals);
    assert_eq!(by_model.previous, exact.previous);
    assert_eq!(by_model.series, exact.series);
    assert_eq!(by_model.projects, exact.projects);
    assert_eq!(by_model.models, exact.models);
    assert_eq!(by_model.agents, exact.agents);
}

#[test]
fn splits_the_figures_by_the_agent_each_session_ran() {
    let summary = by_agent(&mixed(), None, Measure::Tokens);
    let keys: Vec<&str> = summary.agents.iter().map(|row| row.key.as_str()).collect();
    assert_eq!(keys, ["claude", "codex"]);
    let (claude, codex) = (&summary.agents[0], &summary.agents[1]);
    assert_eq!(claude.label, "Claude Code");
    assert_eq!(codex.label, "Codex");
    assert_eq!((claude.totals.tokens, codex.totals.tokens), (2980, 2000));
    assert_eq!((claude.totals.cost, codex.totals.cost), (4.0, 2.0));
    assert_eq!((claude.totals.calls, codex.totals.calls), (4, 4));
    assert_eq!((claude.totals.sessions, codex.totals.sessions), (1, 2));
    assert!((claude.share - 2980.0 / 4980.0).abs() < 1e-12);
    assert!((codex.share - 2000.0 / 4980.0).abs() < 1e-12);

    // The rows add up to the page's totals.
    assert_eq!(
        summary.totals.tokens,
        claude.totals.tokens + codex.totals.tokens
    );
    assert_eq!(summary.totals.cost, claude.totals.cost + codex.totals.cost);
    assert_eq!(
        summary.totals.sessions,
        claude.totals.sessions + codex.totals.sessions
    );
    let shares: f64 = summary.agents.iter().map(|row| row.share).sum();
    assert!((shares - 1.0).abs() < 1e-12);
}

#[test]
fn ranks_the_agents_by_the_measure_picked() {
    let mut data = mixed();
    // Codex cost the most, Claude Code moved the most tokens.
    data.stats.buckets[2].cost = 9.0;
    let tokens = by_agent(&data, None, Measure::Tokens);
    assert_eq!(tokens.agents[0].key, "claude");
    let cost = by_agent(&data, None, Measure::Cost);
    assert_eq!(cost.agents[0].key, "codex");
    assert!(cost.agents[0].share > cost.agents[1].share);
}

#[test]
fn a_project_filter_narrows_the_agents_too() {
    let summary = by_agent(&mixed(), Some("dir:c:/dev/shop"), Measure::Tokens);
    assert_eq!(summary.agents.len(), 1);
    let codex = &summary.agents[0];
    assert_eq!(codex.key, "codex");
    assert_eq!(codex.share, 1.0);
    assert_eq!(codex.totals, summary.totals);

    let app = by_agent(&mixed(), Some("ws:app"), Measure::Tokens);
    assert_eq!(agent_row(&app, "claude").totals.sessions, 1);
    assert_eq!(agent_row(&app, "codex").totals.sessions, 1);
}

#[test]
fn says_which_agents_ran_a_model_without_a_price() {
    let summary = by_agent(&mixed(), None, Measure::Tokens);
    assert!(!agent_row(&summary, "claude").unpriced);
    assert!(agent_row(&summary, "codex").unpriced);
    assert_eq!(summary.unpriced_tokens, 1000);
    // The models of both kinds are ranked together, each by its own name.
    let labels: Vec<(&str, &str)> = summary
        .models
        .iter()
        .map(|row| (row.label.as_str(), row.detail.as_str()))
        .collect();
    assert!(labels.contains(&("Opus 5.5", "claude-opus-5-5")));
    assert!(labels.contains(&("GPT-5.3 Codex", "gpt-5.3-codex")));
    assert!(labels.contains(&("GPT-5.5 Codex", "gpt-5.5-codex")));
}

#[test]
fn a_session_the_list_does_not_name_counts_for_the_default_agent() {
    let mut data = mixed();
    data.session_agents.truncate(1);
    let summary = by_agent(&data, None, Measure::Tokens);
    assert_eq!(summary.agents.len(), 1);
    assert_eq!(summary.agents[0].key, "claude");
    assert_eq!(summary.agents[0].totals.tokens, summary.totals.tokens);
}

#[test]
fn without_the_agents_of_the_sessions_each_row_goes_to_the_maker_of_its_model() {
    let data = mixed();
    let projects = vec![
        projects()[0].clone(),
        projects()[1].clone(),
        projects()[0].clone(),
    ];
    let read = |summary: Summary| summary.agents;
    let by_model = read(summarize(
        &data.stats,
        &SummaryOptions {
            period: Period::D7,
            project: None,
            measure: Measure::Tokens,
            now: now(),
            projects: &projects,
            top_sessions: TOP_SESSIONS,
        },
    ));
    let exact = read(by_agent(&data, None, Measure::Tokens));
    assert_eq!(by_model, exact);
}
