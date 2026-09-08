use crate::job_queue::{IpcRequest, IpcResponse, Job, JobType, send_ipc_request};
use postkit::rest_api::{Request, RestServer, RouteResponse};
use std::net::TcpListener;

/// The one path an API key is not required on.
const HEALTH_PATH: &str = "/health";

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Bind the API without serving it, so a caller can read the address it got when
/// the port was 0.
pub fn bind_rest_api(
    bind_addr: &str,
    api_key: Option<&str>,
) -> Result<(RestServer, TcpListener), String> {
    let server = build_server(bind_addr, api_key);
    let listener = server
        .bind()
        .map_err(|e| format!("Failed to bind to {bind_addr}: {e}"))?;
    Ok((server, listener))
}

/// Start a REST API server for DCP operations (blocking).
///
/// Endpoints:
/// - `GET  /health`        — health check
/// - `GET  /daemon-status` — whether the job daemon answers
/// - `GET  /jobs`          — list the daemon's jobs
/// - `POST /create`        — submit a `DcpConfig` as a create job
/// - `POST /verify`        — submit a DCP path as a verify job
/// - `GET  /metrics`       — Prometheus metrics
pub fn start_rest_api(bind_addr: &str, api_key: Option<&str>) -> i32 {
    let (server, listener) = match bind_rest_api(bind_addr, api_key) {
        Ok(bound) => bound,
        Err(e) => {
            tracing::error!("{e}");
            return -1;
        }
    };

    if !crate::job_queue::is_daemon_running() {
        tracing::warn!(
            "job daemon is not running; job routes will 503. Start it with: dcpwizard daemon"
        );
    }

    tracing::info!("REST API listening on {bind_addr}");

    match server.serve_forever(listener) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("REST API server failed: {e}");
            -1
        }
    }
}

// this server owns no queue of its own: every job route proxies to the shared job
// daemon over IPC, the same queue the `batch` CLI drives
fn build_server(bind_addr: &str, api_key: Option<&str>) -> RestServer {
    let mut server = RestServer::new(bind_addr);
    if let Some(key) = api_key {
        server.require_api_key(key, &[HEALTH_PATH]);
    }

    server.route(
        "GET",
        HEALTH_PATH,
        Box::new(|_request| (200, serde_json::json!({"status": "ok"}).to_string())),
    );

    server.route(
        "GET",
        "/daemon-status",
        Box::new(|_request| {
            let running = crate::job_queue::is_daemon_running();
            (
                200,
                serde_json::json!({"daemon_running": running}).to_string(),
            )
        }),
    );

    server.route(
        "GET",
        "/jobs",
        Box::new(|_request| match daemon_jobs() {
            Ok(jobs) => (
                200,
                serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".into()),
            ),
            Err(e) => (503, daemon_error(&e)),
        }),
    );

    server.route("POST", "/create", Box::new(create_job));
    server.route("POST", "/verify", Box::new(verify_job));

    server.route_with_content_type(
        "GET",
        "/metrics",
        Box::new(|_request| match daemon_jobs() {
            Ok(jobs) => RouteResponse {
                status: 200,
                content_type: PROMETHEUS_CONTENT_TYPE,
                body: build_prometheus_metrics(&jobs),
            },
            Err(e) => RouteResponse::json(503, daemon_error(&e)),
        }),
    );

    server
}

// unknown fields in the body are ignored, the policy DcpConfig's Deserialize has
fn create_job(request: &Request) -> (u16, String) {
    if let Err(e) = serde_json::from_str::<crate::dcp::DcpConfig>(&request.body) {
        return (
            400,
            serde_json::json!({"error": format!("Invalid config: {e}")}).to_string(),
        );
    }
    submitted(JobType::CreateDcp, &request.body)
}

fn verify_job(request: &Request) -> (u16, String) {
    let path = request.body.trim().trim_matches('"');
    if path.is_empty() {
        return (
            400,
            serde_json::json!({"error": "Missing DCP path in body"}).to_string(),
        );
    }
    submitted(JobType::VerifyDcp, path)
}

fn submitted(job_type: JobType, params: &str) -> (u16, String) {
    match submit_to_daemon(job_type, params) {
        Ok(job_id) => (202, serde_json::json!({"job_id": job_id}).to_string()),
        Err(e) => (503, daemon_error(&e)),
    }
}

/// Ask the daemon for the current job list over IPC.
fn daemon_jobs() -> Result<Vec<Job>, String> {
    match send_ipc_request(&IpcRequest::List)? {
        IpcResponse::Jobs(jobs) => Ok(jobs),
        IpcResponse::Error(e) => Err(e),
        _ => Err("unexpected daemon response".into()),
    }
}

/// Submit a job to the daemon over IPC, returning the new job id.
fn submit_to_daemon(job_type: JobType, params: &str) -> Result<String, String> {
    match send_ipc_request(&IpcRequest::Submit {
        job_type,
        params: params.to_string(),
    })? {
        IpcResponse::Submitted { id } => Ok(id),
        IpcResponse::Error(e) => Err(e),
        _ => Err("unexpected daemon response".into()),
    }
}

fn daemon_error(e: &str) -> String {
    serde_json::json!({
        "error": format!("job daemon unavailable: {e}. Start it with: dcpwizard daemon")
    })
    .to_string()
}

/// Build Prometheus-compatible metrics text from a job list.
fn build_prometheus_metrics(jobs: &[Job]) -> String {
    use crate::job_queue::JobState;
    use std::fmt::Write;

    let total = jobs.len();
    let pending = jobs.iter().filter(|j| j.state == JobState::Pending).count();
    let running = jobs.iter().filter(|j| j.state == JobState::Running).count();
    let completed = jobs
        .iter()
        .filter(|j| j.state == JobState::Completed)
        .count();
    let failed = jobs.iter().filter(|j| j.state == JobState::Failed).count();

    let mut out = String::new();

    let _ = writeln!(
        out,
        "# HELP dcpwizard_jobs_total Total number of jobs submitted."
    );
    let _ = writeln!(out, "# TYPE dcpwizard_jobs_total gauge");
    let _ = writeln!(out, "dcpwizard_jobs_total {total}");
    let _ = writeln!(out);
    let _ = writeln!(out, "# HELP dcpwizard_jobs_pending Number of pending jobs.");
    let _ = writeln!(out, "# TYPE dcpwizard_jobs_pending gauge");
    let _ = writeln!(out, "dcpwizard_jobs_pending {pending}");
    let _ = writeln!(out);
    let _ = writeln!(out, "# HELP dcpwizard_jobs_running Number of running jobs.");
    let _ = writeln!(out, "# TYPE dcpwizard_jobs_running gauge");
    let _ = writeln!(out, "dcpwizard_jobs_running {running}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "# HELP dcpwizard_jobs_completed Number of completed jobs."
    );
    let _ = writeln!(out, "# TYPE dcpwizard_jobs_completed gauge");
    let _ = writeln!(out, "dcpwizard_jobs_completed {completed}");
    let _ = writeln!(out);
    let _ = writeln!(out, "# HELP dcpwizard_jobs_failed Number of failed jobs.");
    let _ = writeln!(out, "# TYPE dcpwizard_jobs_failed gauge");
    let _ = writeln!(out, "dcpwizard_jobs_failed {failed}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "# HELP dcpwizard_daemon_running Whether the job daemon is running."
    );
    let _ = writeln!(out, "# TYPE dcpwizard_daemon_running gauge");
    let daemon_up = if crate::job_queue::is_daemon_running() {
        1
    } else {
        0
    };
    let _ = writeln!(out, "dcpwizard_daemon_running {daemon_up}");

    out
}
