mod crossterm_terminal;
mod history;
mod history_prompt;

use anyhow::{Context, Result};
use std::sync::Arc;
use terminal_cli::{
    validate_property_min_max, CharacterTerminalReader, CharacterTerminalWriter, CliContext,
    CliExecutor, PromptEvent, PropertyContext,
};

use decision_core::engine::DecisionEngine;
use decision_core::models::traits::*;

use crate::crossterm_terminal::CrosstermTerminal;
use crate::history::History;
use crate::history_prompt::HistoryPrompt;

// ──────────────────────────────────────────────────────────
// 1. DevState — ค่าที่ปรับได้ระหว่าง dev
// ──────────────────────────────────────────────────────────
#[derive(Clone)]
struct DevState {
    cascade_threshold: f64, // 0.0–1.0
    laya_conf_min: f64,     // 0.0–1.0
    evolve_enabled: i32,    // 0|1
    log_level: i32,         // 0=error,1=warn,2=info,3=debug
    active_backend: String, // string → ใช้ command ไม่ใช่ property
    #[allow(dead_code)]
    last_decision_id: Option<i64>,
    should_quit: bool, // ให้ REPL ออกเอง เพื่อคืนค่า terminal ให้ถูกต้อง (แทน process::exit)
}

impl Default for DevState {
    fn default() -> Self {
        Self {
            cascade_threshold: 0.60,
            laya_conf_min: 0.60,
            evolve_enabled: 1,
            log_level: 2,
            active_backend: "laya".into(),
            last_decision_id: None,
            should_quit: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// 2. REPL loop
// ──────────────────────────────────────────────────────────
fn run_repl(engine: Arc<DecisionEngine>, state: &mut DevState) -> Result<()> {
    let history = History::load(History::default_path(), 1000);
    let mut prompt = HistoryPrompt::new("decision-dev> ", history);
    let mut terminal =
        CrosstermTerminal::new().context("ต้องรันใน terminal จริง (stdin ต้องเป็น TTY)")?;

    prompt.print_prompt(&mut terminal);

    loop {
        let key = match terminal.read() {
            Ok(k) => k,
            Err(_) => break,
        };
        // handle_key พิมพ์ prompt ใหม่หลังรันคำสั่งเอง → ไม่ต้องเรียก print_prompt ซ้ำ
        let ev = prompt.handle_key(key, &mut terminal, |m| dispatch(m, state, &engine));
        if ev == PromptEvent::Break || state.should_quit {
            break;
        }
    }

    terminal.print_line("");
    Ok(())
}

// ──────────────────────────────────────────────────────────
// 3. Dispatch — จุดเดียวที่ต้องแก้เวลาเพิ่ม command
// ──────────────────────────────────────────────────────────
fn dispatch(m: &mut CliExecutor, state: &mut DevState, engine: &Arc<DecisionEngine>) {
    if let Some(mut ctx) = m.command("help") {
        print_help(ctx.get_terminal());
    }

    if m.command("quit").is_some() || m.command("exit").is_some() {
        state.should_quit = true;
    }

    if let Some(mut sub) = m.with_prefix("backend/") {
        if let Some(mut ctx) = sub.command("list") {
            for name in engine.registry.names() {
                ctx.get_terminal().print_line(&format!("  {}", name));
            }
        }
        if let Some(mut ctx) = sub.command("select") {
            let want = ctx.get_args().trim().to_string();
            match engine.registry.get(Some(&want)) {
                Ok(b) => {
                    state.active_backend = b.name().to_string();
                    ctx.get_terminal().print_line(&format!("backend → {}", want));
                }
                Err(e) => ctx.get_terminal().print_line(&format!("error: {e}")),
            }
        }
    }

    if let Some(mut ctx) = m.command("decide") {
        let args = ctx.get_args().to_string();
        match run_decide(engine, state, &args) {
            Ok(msg) => ctx.get_terminal().print_line(&msg),
            Err(e) => ctx.get_terminal().print_line(&format!("error: {e:#}")),
        }
    }

    if let Some(mut ctx) = m.command("history") {
        let q = ctx.get_args().to_string();
        ctx.get_terminal().print_line(&format!("[stub] search: {q}"));
    }

    if let Some(mut sub) = m.with_prefix("store/") {
        if let Some(mut ctx) = sub.command("count") {
            ctx.get_terminal().print_line("[stub] count");
        }
        if let Some(mut ctx) = sub.command("last") {
            let n: i64 = ctx.get_args().trim().parse().unwrap_or(5);
            ctx.get_terminal().print_line(&format!("[stub] last {n}"));
        }
        if let Some(mut ctx) = sub.command("get") {
            let id: i64 = ctx.get_args().trim().parse().unwrap_or(0);
            ctx.get_terminal().print_line(&format!("[stub] get #{id}"));
        }
    }

    if let Some(mut ctx) = m.command("teach") {
        let id: i64 = ctx.get_args().trim().parse().unwrap_or(0);
        ctx.get_terminal().print_line(&format!("[stub] teach #{id}"));
    }

    if let Some(mut sub) = m.with_prefix("weights/") {
        if let Some(mut ctx) = sub.command("list") {
            let scope = ctx.get_args().trim().to_string();
            ctx.get_terminal().print_line(&format!("[stub] weights in {scope}"));
        }
    }

    if let Some(mut sub) = m.with_prefix("rune/") {
        if let Some(mut ctx) = sub.command("list") {
            ctx.get_terminal().print_line("[stub] rune list");
        }
        if let Some(mut ctx) = sub.command("reload") {
            ctx.get_terminal().print_line("[stub] reloaded");
        }
    }

    if let Some(mut sub) = m.with_prefix("policy/") {
        if let Some(mut ctx) = sub.command("list") {
            ctx.get_terminal().print_line("[stub] policy list");
        }
        if let Some(mut ctx) = sub.command("eval") {
            let j = ctx.get_args().to_string();
            ctx.get_terminal().print_line(&format!("[stub] eval {j}"));
        }
    }

    if let Some(mut sub) = m.with_prefix("evolve/") {
        if let Some(mut ctx) = sub.command("status") {
            ctx.get_terminal().print_line("[stub] evolution status");
        }
        if let Some(mut ctx) = sub.command("run") {
            let scope = ctx.get_args().trim().to_string();
            ctx.get_terminal().print_line(&format!("[stub] evolve {scope}"));
        }
    }

    if let Some(mut ctx) = m.command("bench") {
        ctx.get_terminal().print_line("[stub] benchmark");
    }

    // ── Properties (<id>/get, <id>/set <val>) — apply() ต้องใช้ V: Display + Copy ──
    if let Some(mut ctx) = m.property("cascade/threshold", validate_property_min_max(0, 100)) {
        let mut scaled = (state.cascade_threshold * 100.0) as i32;
        ctx.apply(&mut scaled);
        if matches!(ctx, PropertyContext::Set(_)) {
            state.cascade_threshold = scaled as f64 / 100.0;
        }
    }

    if let Some(mut ctx) = m.property("laya/conf_min", validate_property_min_max(0, 100)) {
        let mut scaled = (state.laya_conf_min * 100.0) as i32;
        ctx.apply(&mut scaled);
        if matches!(ctx, PropertyContext::Set(_)) {
            state.laya_conf_min = scaled as f64 / 100.0;
        }
    }

    if let Some(mut ctx) = m.property("evolve/enabled", validate_property_min_max(0, 1)) {
        ctx.apply(&mut state.evolve_enabled);
    }

    if let Some(mut ctx) = m.property("log/level", validate_property_min_max(0, 3)) {
        ctx.apply(&mut state.log_level);
    }
}

// ──────────────────────────────────────────────────────────
// 4. Handler — decide
// ──────────────────────────────────────────────────────────
fn run_decide(engine: &Arc<DecisionEngine>, state: &mut DevState, raw_args: &str) -> Result<String> {
    let parts: Vec<String> = shell_words::split(raw_args)?;
    if parts.len() < 3 {
        anyhow::bail!("usage: decide <scope> <kind> \"<context>\"");
    }

    let scope = &parts[0];
    let kind_str = &parts[1];
    let context = parts[2..].join(" ");

    let backend = engine.registry.get(Some(&state.active_backend))?;

    let spec = match kind_str.as_str() {
        "content" => QuestionSpec {
            name: "answer".into(),
            kind: QuestionKind::Noul { instructions: context.clone() },
        },
        "score" => QuestionSpec {
            name: "quality".into(),
            kind: QuestionKind::Score {
                instructions: context.clone(),
                levels: vec!["bad".into(), "ok".into(), "good".into()],
            },
        },
        other => anyhow::bail!("unknown kind: {other}"),
    };

    let req = DecisionRequest {
        state: serde_json::json!({ "scope": scope, "context": context }),
        questions: vec![spec],
    };

    // handler รันบน spawn_blocking → block_on บน runtime handle ได้
    let rt = tokio::runtime::Handle::current();
    let resp = rt.block_on(backend.decide(req))?;

    let mut out = String::new();
    out.push_str(&format!("backend={} latency={}ms\n", resp.backend, resp.latency_ms));
    for a in &resp.answers {
        out.push_str(&format!("  {} → {:?}\n", a.name, a.answer));
    }
    Ok(out)
}

// ──────────────────────────────────────────────────────────
// 5. Help
// ──────────────────────────────────────────────────────────
fn print_help(t: &mut dyn CharacterTerminalWriter) {
    for line in &[
        "backend/list | backend/select <name>",
        "decide <scope> <kind> \"<context>\"",
        "history <query>",
        "store/count | store/last [N] | store/get <id>",
        "teach <decision_id>",
        "weights/list <scope>",
        "rune/list | rune/reload",
        "policy/list | policy/eval '<json>'",
        "evolve/status | evolve/run <scope>",
        "bench",
        "",
        "Properties: cascade/threshold  laya/conf_min  evolve/enabled  log/level",
        "  syntax: <prop>/get  |  <prop>/set <value>",
        "Keys: ↑/↓ history  Tab autocomplete  Ctrl+C ล้างบรรทัด  Ctrl+D ออก",
        "quit",
    ] {
        t.print_line(line);
    }
}

// ──────────────────────────────────────────────────────────
// 6. Entry point
// ──────────────────────────────────────────────────────────
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let engine = Arc::new(decision_core::build_engine().await?);
    let mut state = DevState::default();

    let engine_clone = engine.clone();
    tokio::task::spawn_blocking(move || run_repl(engine_clone, &mut state)).await??;

    Ok(())
}
