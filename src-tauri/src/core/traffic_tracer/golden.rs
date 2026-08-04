use std::{
    env, fs,
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::protocol::{EmptyParams, Request, RequestMethod, WORKER_API_VERSION};

#[derive(Debug, Deserialize)]
struct GoldenEvent {
    schema_version: u32,
    session_id: String,
    event_seq: u64,
    #[serde(rename = "type")]
    kind: String,
    network: String,
    pre_flow: Option<GoldenFlow>,
    post_flow: Option<GoldenFlow>,
    status: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoldenFlow {
    network: String,
    src_ip: IpAddr,
    src_port: u16,
    dst_ip: IpAddr,
    dst_port: u16,
    key: String,
    complete: bool,
    shared: bool,
}

#[derive(Default, Debug)]
struct Coverage {
    tcp: bool,
    udp: bool,
    ipv4: bool,
    ipv6: bool,
    shared: bool,
    error: bool,
}

fn endpoint(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(ip) => format!("{ip}:{port}"),
        IpAddr::V6(ip) => format!("[{ip}]:{port}"),
    }
}

fn validate_flow(flow: GoldenFlow, coverage: &mut Coverage) {
    coverage.ipv4 |= flow.src_ip.is_ipv4() || flow.dst_ip.is_ipv4();
    coverage.ipv6 |= flow.src_ip.is_ipv6() || flow.dst_ip.is_ipv6();
    coverage.shared |= flow.shared;
    if flow.complete {
        assert_eq!(
            flow.key,
            format!(
                "{}|{}|{}",
                flow.network,
                endpoint(flow.src_ip, flow.src_port),
                endpoint(flow.dst_ip, flow.dst_port)
            )
        );
    } else {
        assert!(flow.key.is_empty());
    }
}

fn golden_files(root: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(root)
        .expect("shared golden directory should be readable")
        .map(|entry| entry.expect("golden directory entry should be readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn golden_complete_tracing_events() {
    let root = env::var_os("TRAFFICTRACER_GOLDEN_DIR")
        .map(PathBuf::from)
        .expect("TRAFFICTRACER_GOLDEN_DIR is required for the cross-language contract gate");
    let paths = golden_files(&root);
    assert!(!paths.is_empty(), "no shared tracing fixtures found");

    let mut coverage = Coverage::default();
    for path in paths {
        let mut previous = 0;
        for line in fs::read_to_string(&path)
            .expect("golden fixture should be readable")
            .lines()
        {
            let event: GoldenEvent = serde_json::from_str(line).expect("golden event should deserialize");
            assert_eq!(event.schema_version, 1);
            assert!(!event.session_id.is_empty());
            assert!(event.event_seq > previous);
            previous = event.event_seq;
            assert!(matches!(
                event.kind.as_str(),
                "tcp_connect"
                    | "tcp_proxy_dial"
                    | "tcp_close"
                    | "udp_connect"
                    | "udp_proxy_dial"
                    | "udp_out"
                    | "udp_in"
                    | "udp_close"
            ));
            match event.network.as_str() {
                "tcp" => coverage.tcp = true,
                "udp" => coverage.udp = true,
                network => panic!("unsupported golden network: {network}"),
            }
            coverage.error |= event.status.as_deref() == Some("dial_error")
                && event.error.as_deref().is_some_and(|error| !error.is_empty());
            if let Some(flow) = event.pre_flow {
                validate_flow(flow, &mut coverage);
            }
            if let Some(flow) = event.post_flow {
                validate_flow(flow, &mut coverage);
            }
        }
    }

    assert!(coverage.tcp, "{coverage:?}");
    assert!(coverage.udp, "{coverage:?}");
    assert!(coverage.ipv4, "{coverage:?}");
    assert!(coverage.ipv6, "{coverage:?}");
    assert!(coverage.shared, "{coverage:?}");
    assert!(coverage.error, "{coverage:?}");
}

#[test]
fn golden_complete_worker_batch_request() {
    let root = env::var_os("TRAFFICTRACER_CONTRACT_DIR")
        .map(PathBuf::from)
        .expect("TRAFFICTRACER_CONTRACT_DIR is required for the cross-language contract gate");
    let raw = fs::read_to_string(root.join("worker-request-batch-valid.json"))
        .expect("shared batch Worker fixture should be readable");
    let request: Request<EmptyParams> =
        serde_json::from_str(&raw).expect("shared batch Worker fixture should deserialize");
    assert_eq!(request.api_version, WORKER_API_VERSION);
    assert_eq!(request.method, RequestMethod::BatchList);
}
