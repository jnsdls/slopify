// Throwaway spike for jnsdls/slopify#22: can the Web Playback SDK play inside a WKWebView (wry) with
// Apple's FairPlay CDM and no Widevine? Prints `RESULT: PLAYED <n>s` or `RESULT: FAILED <reason>`.
//
//   cargo run --release -- [--hidden] [--origin custom|localhost] [--throttling default|disabled] [--no-autoplay]

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::{BackgroundThrottlingPolicy, WebViewBuilder};

const CLIENT_ID: &str = "a768335a56b648d4a6d11d945d029ce4";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const KEYCHAIN_SERVICE: &str = "slopify-spotify-refresh-token";
const KEYCHAIN_ACCOUNT: &str = "1143227716";
const SECURITY: &str = "/usr/bin/security";
const TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
const PAGE: &str = include_str!("page.html");
const INJECT: &str = include_str!("inject.js");

const READY_TIMEOUT: Duration = Duration::from_secs(45);
const START_TIMEOUT: Duration = Duration::from_secs(30);
const PLAY_FOR: Duration = Duration::from_secs(33);

static LOG: OnceLock<Mutex<File>> = OnceLock::new();
static T0: OnceLock<Instant> = OnceLock::new();

fn log(line: impl AsRef<str>) {
    let t = T0.get().map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
    let mut line = line.as_ref().to_string();
    if line.len() > 900 {
        let mut cut = 900;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
        line.push_str(" …");
    }
    let line = format!("[{t:7.2}] {line}");
    println!("{line}");
    if let Some(f) = LOG.get() {
        let _ = writeln!(f.lock().unwrap(), "{line}");
    }
}

enum UserEvent {
    Eval(String),
    Exit(i32),
}

#[derive(Default)]
struct Shared {
    device_id: Option<String>,
    paused: Option<bool>,
    max_position_ms: u64,
    last_state_paused_at: Option<Instant>,
    last_state_playing_at: Option<Instant>,
    sdk_errors: Vec<String>,
    licence: Vec<String>,
    key_systems: Vec<String>,
}

fn main() {
    T0.set(Instant::now()).unwrap();
    let args: Vec<String> = std::env::args().collect();
    let hidden = args.iter().any(|a| a == "--hidden");
    let opt = |name: &str, default: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| default.to_string())
    };
    let origin = opt("--origin", "custom");
    let throttling = opt("--throttling", "default");
    let autoplay = !args.iter().any(|a| a == "--no-autoplay");

    let log_path = concat!(env!("CARGO_MANIFEST_DIR"), "/spike.log");
    LOG.set(Mutex::new(OpenOptions::new().create(true).append(true).open(log_path).unwrap()))
        .unwrap();
    log(format!("==== run: hidden={hidden} origin={origin} throttling={throttling} autoplay={autoplay} pid={}", std::process::id()));

    let token = match access_token() {
        Ok(t) => t,
        Err(e) => {
            log(format!("RESULT: FAILED auth: {e}"));
            std::process::exit(2);
        }
    };

    let webkit_before = webkit_pids();
    let shared = Arc::new(Mutex::new(Shared::default()));

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    if hidden {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("slopify wkwebview spike")
        .with_inner_size(tao::dpi::LogicalSize::new(560.0, 360.0))
        .with_visible(!hidden)
        .build(&event_loop)
        .unwrap();

    let url = match origin.as_str() {
        "localhost" => format!("http://127.0.0.1:{}/", serve_localhost()),
        _ => "spike://localhost/".to_string(),
    };
    log(format!("loading {url}"));

    let token_js = format!("window.__SPIKE_TOKEN = {};", serde_json::to_string(&token).unwrap());
    let ipc_shared = shared.clone();
    let ipc_proxy = proxy.clone();
    let ipc_token = token.clone();
    let mut builder = WebViewBuilder::new()
        .with_url(url)
        .with_autoplay(autoplay)
        .with_devtools(true)
        .with_initialization_script_for_main_only(INJECT, false)
        .with_initialization_script_for_main_only(token_js, true)
        .with_custom_protocol("spike".into(), |_id, _req| {
            http_response(PAGE.as_bytes())
        })
        .with_ipc_handler(move |req| on_ipc(req.body(), &ipc_shared, &ipc_proxy, &ipc_token));
    if throttling == "disabled" {
        builder = builder.with_background_throttling(BackgroundThrottlingPolicy::Disabled);
    }
    let webview = builder.build(&window).unwrap();

    {
        let shared = shared.clone();
        let proxy = proxy.clone();
        std::thread::spawn(move || controller(shared, proxy, webkit_before));
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Exit(code)) => {
                log(format!("exit {code}"));
                *control_flow = ControlFlow::ExitWithCode(code);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            _ => {}
        }
    });
}

fn http_response(body: &'static [u8]) -> wry::http::Response<Cow<'static, [u8]>> {
    wry::http::Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Cow::Borrowed(body))
        .unwrap()
}

/// Minimal HTTP server so the page can load from http://127.0.0.1, a potentially trustworthy origin.
fn serve_localhost() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PAGE.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(PAGE.as_bytes());
        }
    });
    port
}

fn on_ipc(body: &str, shared: &Arc<Mutex<Shared>>, proxy: &EventLoopProxy<UserEvent>, token: &str) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return log(format!("ipc raw: {body}")),
    };
    let t = v["t"].as_str().unwrap_or("");
    let frame = v["frame"].as_str().unwrap_or("?");
    let mut s = shared.lock().unwrap();
    match t {
        "ready" => {
            let id = v["device_id"].as_str().unwrap_or("").to_string();
            log(format!("SDK ready device_id={id}"));
            s.device_id = Some(id.clone());
            let token = token.to_string();
            let shared = shared.clone();
            let proxy = proxy.clone();
            std::thread::spawn(move || start_playback(&token, &id, &shared, &proxy));
        }
        "state" => {
            if v["none"].as_bool() == Some(true) {
                log("SDK player_state_changed: null");
                return;
            }
            let paused = v["paused"].as_bool().unwrap_or(true);
            let pos = v["position"].as_u64().unwrap_or(0);
            log(format!(
                "SDK player_state_changed paused={paused} position={pos} duration={} track={}",
                v["duration"], v["track"]
            ));
            track_position(&mut s, paused, pos);
            if paused {
                s.last_state_paused_at = Some(Instant::now());
            } else {
                s.last_state_playing_at = Some(Instant::now());
            }
        }
        "pos" => {
            let paused = v["paused"].as_bool().unwrap_or(true);
            let pos = v["position"].as_u64().unwrap_or(0);
            track_position(&mut s, paused, pos);
            if pos / 1000 % 5 == 0 {
                log(format!("getCurrentState paused={paused} position={pos}"));
            }
        }
        "sdk-error" => {
            let e = format!("{}: {}", v["event"].as_str().unwrap_or("?"), v["message"].as_str().unwrap_or(""));
            log(format!("SDK ERROR {e}"));
            s.sdk_errors.push(e);
        }
        "net" => {
            let url = v["url"].as_str().unwrap_or("");
            let line = format!(
                "net [{frame}] {} {} {} -> {}",
                v["kind"].as_str().unwrap_or(""),
                v["method"].as_str().unwrap_or(""),
                url,
                if v["error"].is_null() { v["status"].to_string() } else { v["error"].to_string() }
            );
            let lower = url.to_ascii_lowercase();
            if lower.contains("license") || lower.contains("licence") || lower.contains("fairplay") || lower.contains("certificate") {
                s.licence.push(line.clone());
                log(format!("LICENCE {line}"));
            } else {
                log(line);
            }
        }
        "eme" => {
            if ["granted", "request", "denied", "legacy WebKitMediaKeys"].iter().any(|op| v["op"] == *op) {
                s.key_systems.push(format!("{} {}", v["op"].as_str().unwrap_or(""), v["keySystem"].as_str().unwrap_or("")));
            }
            log(format!("EME [{frame}] {v}"));
        }
        _ => log(format!("{t} [{frame}] {v}")),
    }
}

fn track_position(s: &mut Shared, paused: bool, pos: u64) {
    s.paused = Some(paused);
    if pos > s.max_position_ms {
        s.max_position_ms = pos;
    }
}

fn start_playback(token: &str, device_id: &str, _shared: &Arc<Mutex<Shared>>, _proxy: &EventLoopProxy<UserEvent>) {
    let agent = agent();
    let url = format!("https://api.spotify.com/v1/me/player/play?device_id={device_id}");
    let body = serde_json::json!({ "uris": [TRACK] }).to_string();
    match agent
        .put(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(body)
    {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            log(format!("PUT /v1/me/player/play -> {status} {}", text.trim()));
        }
        Err(e) => log(format!("PUT /v1/me/player/play failed: {e}")),
    }
}

fn controller(shared: Arc<Mutex<Shared>>, proxy: EventLoopProxy<UserEvent>, webkit_before: HashSet<u32>) {
    let fail = |reason: String| {
        let s = shared.lock().unwrap();
        summary(&s);
        log(format!("RESULT: FAILED {reason}"));
        let _ = proxy.send_event(UserEvent::Exit(1));
    };
    let wait_until = |timeout: Duration, f: &dyn Fn(&Shared) -> bool| {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if f(&shared.lock().unwrap()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    };

    if !wait_until(READY_TIMEOUT, &|s| s.device_id.is_some() || !s.sdk_errors.is_empty()) {
        return fail("no SDK ready within 45s".into());
    }
    if shared.lock().unwrap().device_id.is_none() {
        let e = shared.lock().unwrap().sdk_errors.join("; ");
        return fail(format!("SDK error before ready: {e}"));
    }
    if !wait_until(START_TIMEOUT, &|s| s.paused == Some(false) && s.max_position_ms > 0) {
        return fail("playback never advanced within 30s of ready".into());
    }
    let started = Instant::now();
    log("playback advancing");

    // Play for PLAY_FOR, sampling memory and Now Playing halfway and at the end.
    std::thread::sleep(PLAY_FOR / 2);
    memory(&webkit_before);
    std::thread::sleep(PLAY_FOR - PLAY_FOR / 2);
    let max = shared.lock().unwrap().max_position_ms;
    log(format!("after {:?} of playback max position {max} ms", started.elapsed()));
    memory(&webkit_before);

    // Now Playing: only send the toggle if the Now Playing owner is us. Otherwise the command lands in
    // whatever app does own it (e.g. a paused Chrome tab) and starts that instead.
    let np = now_playing();
    let our_pids: HashSet<u32> = webkit_pids().difference(&webkit_before).copied().chain([std::process::id()]).collect();
    let owner = np.as_ref().and_then(|v| v["processIdentifier"].as_u64()).map(|p| p as u32);
    let ours = owner.map(|p| our_pids.contains(&p)).unwrap_or(false);
    log(format!("NOW PLAYING owner pid={owner:?} ours={ours}"));
    if ours {
        for step in ["toggle 1 (expect pause)", "toggle 2 (expect resume)"] {
            let before = Instant::now();
            log(format!("media-control toggle-play-pause: {step}"));
            let _ = Command::new("media-control").arg("toggle-play-pause").status();
            std::thread::sleep(Duration::from_secs(4));
            let s = shared.lock().unwrap();
            let paused_after = s.last_state_paused_at.map(|t| t > before).unwrap_or(false);
            let played_after = s.last_state_playing_at.map(|t| t > before).unwrap_or(false);
            log(format!("TOGGLE {step}: paused-event={paused_after} playing-event={played_after} paused={:?}", s.paused));
        }
        now_playing();
    } else {
        log("TOGGLE skipped: Now Playing is not owned by this process tree");
    }

    // Pause through the SDK and exit.
    let before = Instant::now();
    let _ = proxy.send_event(UserEvent::Eval("window.spikePause && window.spikePause()".into()));
    let paused = wait_until(Duration::from_secs(5), &|s| s.last_state_paused_at.map(|t| t > before).unwrap_or(false) || s.paused == Some(true));
    log(format!("SDK pause -> paused={paused}"));
    let _ = proxy.send_event(UserEvent::Eval("window.spikeDisconnect && window.spikeDisconnect()".into()));
    std::thread::sleep(Duration::from_millis(800));

    let s = shared.lock().unwrap();
    summary(&s);
    let secs = s.max_position_ms / 1000;
    if !s.sdk_errors.iter().any(|e| e.starts_with("playback_error")) && secs >= 20 {
        log(format!("RESULT: PLAYED {secs}s"));
        let _ = proxy.send_event(UserEvent::Exit(0));
    } else {
        log(format!("RESULT: FAILED max position {secs}s, errors: {:?}", s.sdk_errors));
        let _ = proxy.send_event(UserEvent::Exit(1));
    }
}

fn summary(s: &Shared) {
    log("---- summary ----");
    log(format!("key systems: {:?}", s.key_systems));
    for l in &s.licence {
        log(format!("licence: {l}"));
    }
    log(format!("sdk errors: {:?}", s.sdk_errors));
    log(format!("max position: {} ms", s.max_position_ms));
}

fn now_playing() -> Option<Value> {
    let out = Command::new("media-control").arg("get").output().ok()?;
    let mut v: Value = serde_json::from_slice(&out.stdout).ok()?;
    if let Some(o) = v.as_object_mut() {
        o.retain(|k, _| !k.starts_with("artwork") || k == "artworkMimeType");
    }
    log(format!("media-control get: {v}"));
    Some(v)
}

fn ps_table() -> Vec<(u32, u32, u64, String)> {
    let out = Command::new("ps").args(["-axo", "pid=,ppid=,rss=,comm="]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let rss = it.next()?.parse().ok()?;
            let comm = it.collect::<Vec<_>>().join(" ");
            Some((pid, ppid, rss, comm))
        })
        .collect()
}

fn webkit_pids() -> HashSet<u32> {
    ps_table().into_iter().filter(|p| p.3.contains("com.apple.WebKit")).map(|p| p.0).collect()
}

/// WebKit's helpers are XPC services (ppid 1), so attribute them by diffing against a snapshot taken
/// before the webview was created.
fn memory(before: &HashSet<u32>) {
    let me = std::process::id();
    let mut total = 0;
    for (pid, ppid, rss, comm) in ps_table() {
        let ours = pid == me || (ppid == me && !comm.ends_with("ps")) || (comm.contains("com.apple.WebKit") && !before.contains(&pid));
        if ours {
            total += rss;
            let name = comm.rsplit('/').next().unwrap_or(&comm);
            log(format!("MEM pid={pid} rss={} MB {name}", rss / 1024));
        }
    }
    log(format!("MEM total rss={} MB", total / 1024));
    // coreaudiod holds an audio-out assertion naming the process that is actually producing sound.
    if let Ok(out) = Command::new("pmset").args(["-g", "assertions"]).output() {
        for l in String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("WebKit") || l.contains("wkwebview") || l.to_lowercase().contains("audio")) {
            log(format!("AUDIO {}", l.trim()));
        }
    }
}

// ---- auth ----

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

/// One refresh per run. Spotify rotates the refresh token, so the new one goes straight back into the
/// Keychain item the real app reads, before anything else can fail.
fn access_token() -> Result<String, String> {
    let out = Command::new(SECURITY)
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("keychain read exited {:?}", out.status.code()));
    }
    let refresh = String::from_utf8(out.stdout).map_err(|e| e.to_string())?.trim_end().to_string();
    if refresh.is_empty() {
        return Err("empty refresh token in keychain".into());
    }

    let mut r = agent()
        .post(TOKEN_URL)
        .send_form([("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", CLIENT_ID)])
        .map_err(|e| e.to_string())?;
    let status = r.status().as_u16();
    let body: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("token endpoint {status}: {} {}", body["error"], body["error_description"]));
    }
    if let Some(next) = body["refresh_token"].as_str() {
        if next != refresh {
            write_refresh_token(next)?;
            log("refresh token rotated, written back to keychain and verified");
        } else {
            log("refresh token unchanged");
        }
    } else {
        log("no refresh token in response, keychain untouched");
    }
    log(format!("access token ok, scope: {}", body["scope"].as_str().unwrap_or("?")));
    body["access_token"].as_str().map(str::to_string).ok_or_else(|| "no access_token".into())
}

fn write_refresh_token(token: &str) -> Result<(), String> {
    let quote = |v: &str| format!("'{}'", v.replace('\'', "'\\''"));
    let cmd = format!(
        "add-generic-password -a {} -s {KEYCHAIN_SERVICE} -U -T {SECURITY} -w {}\n",
        quote(KEYCHAIN_ACCOUNT),
        quote(token)
    );
    for attempt in 1..=3 {
        let mut child = Command::new(SECURITY)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child.stdin.take().unwrap().write_all(cmd.as_bytes()).map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let back = Command::new(SECURITY)
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() && String::from_utf8_lossy(&back.stdout).trim_end() == token {
            return Ok(());
        }
        log(format!("keychain write attempt {attempt} failed: {}", String::from_utf8_lossy(&out.stderr).replace(token, "REDACTED").trim()));
    }
    Err("could not write rotated refresh token back to keychain".into())
}
