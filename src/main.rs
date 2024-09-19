use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server, Uri};
use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use tokio::time::{timeout, Duration};
use getopts::Options;

async fn proxy_request(req: Request<Body>, upstream_base_uri: Uri) -> Result<Response<Body>, hyper::Error> {
    // Set up HTTP client (no HTTPS)
    let client = Client::new();
    let req_header_temp = req.headers().clone();
    
    // Create the new URI by merging the upstream base URI with the incoming request's path and query
    let mut parts = upstream_base_uri.into_parts();
    parts.path_and_query = req.uri().path_and_query().cloned();
    let new_uri = Uri::from_parts(parts).expect("valid URI");

    // Create a new request for the upstream WebDAV server
    let mut new_req = Request::builder()
        .method(req.method())
        .uri(new_uri)
        .body(req.into_body())
        .expect("request builder");

    // Copy the headers from the original request
    *new_req.headers_mut() = req_header_temp;

    // Try to forward the request with a timeout (5 seconds for example)
    match timeout(Duration::from_secs(5), client.request(new_req)).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => {
            // Handle port closed case
            create_error_response("closed").await
        }
        Err(_) => {
            // Handle timeout case
            create_error_response("timeout").await
        }
    }
}

// Generate a response as if accessing an empty folder with a "TIMEOUT" or "CLOSED" file
async fn create_error_response(reason: &str) -> Result<Response<Body>, hyper::Error> {
    let body = Body::from(
        format!(r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/{}/</d:href>
    <d:propstat>
      <d:prop>
        <d:displayname>{}</d:displayname>
        <d:resourcetype><d:collection/></d:resourcetype>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#, reason, reason.to_uppercase()),
    );
    Ok(Response::builder()
        .status(207)
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(body)
        .expect("response builder"))
}

fn print_usage(program: &str, opts: Options) {
    let program_path = std::path::PathBuf::from(program);
    let program_name = program_path.file_stem().unwrap().to_string_lossy();
    let brief = format!(
        "Usage: {} REMOTE_HOST:PORT [-b BIND_ADDR] [-l LOCAL_PORT]",
        program_name
    );
    print!("{}", opts.usage(&brief));
}

//Commandline parsing from https://github.com/mqudsi/tcpproxy
#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt(
        "b",
        "bind",
        "The address on which to listen for incoming requests, defaulting to localhost",
        "BIND_ADDR",
    );
    opts.optopt(
        "l",
        "local-port",
        "The local port to which tcpproxy should bind to, randomly chosen otherwise",
        "LOCAL_PORT",
    );

    let matches = match opts.parse(&args[1..]) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("{}", e);
            print_usage(&program, opts);
            std::process::exit(-1);
        }
    };
    let remote = match matches.free.len() {
        1 => matches.free[0].clone(),
        _ => {
            print_usage(&program, opts);
            std::process::exit(-1);
        }
    };

    if !remote.contains(':') {
        eprintln!("A remote port is required (REMOTE_ADDR:PORT)");
        std::process::exit(-1);
    }

    // let local_port: i32 = matches.opt_str("l").unwrap_or("0".to_string()).parse()?;
    let local_port: u16 = matches.opt_str("l").map(|s| s.parse()).unwrap_or(Ok(0)).expect("aga");
    let bind_addr = match matches.opt_str("b") {
        Some(addr) => addr.parse::<std::net::IpAddr>().expect("Failed to parse bind address"),
        None => std::net::IpAddr::from([127, 0, 0, 1]),
    };

    // Define the upstream WebDAV server base URL (using HTTP)
    let upstream_uri = format!("http://{}",remote).parse::<Uri>().unwrap();
    println!("The upstream is http://{}", remote);

    // Define the proxy service
    let make_svc = make_service_fn(|_conn| {
        let upstream_uri = upstream_uri.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                proxy_request(req, upstream_uri.clone())
            }))
        }
    });

    // Define the address and port to listen on
    let addr = SocketAddr::new(bind_addr, local_port);

    // Create the server
    let server = Server::bind(&addr).serve(make_svc);

    println!("Listening on http://{}", addr);

    // Run the server
    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }
}
