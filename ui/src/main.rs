#![feature(try_from)]

#[macro_use]
extern crate log;
extern crate env_logger;
extern crate iron;
extern crate mount;
extern crate playground_middleware;
extern crate bodyparser;
extern crate hyper_native_tls;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate mktemp;
#[macro_use]
extern crate quick_error;

use std::any::Any;
use std::convert::{TryFrom, TryInto};
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use iron::headers::ContentType;
use iron::modifiers::Header;
use iron::prelude::*;
use iron::status;

use mount::Mount;
use serde::{Serialize, Deserialize};
use playground_middleware::{
    Cache,
    FileLogger,
    GuessContentType,
    HttpToHttpsRedirect,
    ModifyWith,
    Prefix,
    Rewrite,
    Staticfile,
    StatisticLogger,
};
use hyper_native_tls::NativeTlsServer;

use sandbox::Sandbox;

const DEFAULT_LISTEN_HOST: &'static str = "127.0.0.1";
const DEFAULT_HTTP_PORT: u16 = 5000;
const DEFAULT_HTTPS_PORT: u16 = 5001;
const DEFAULT_LOG_FILE: &'static str = "access-log.csv";

mod sandbox;

const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;
const ONE_YEAR_IN_SECONDS: u64 = 60 * 60 * 24 * 365;

fn main() {
    env_logger::init().expect("Unable to initialize logger");

    let root: PathBuf = env::var_os("PLAYGROUND_UI_ROOT").expect("Must specify PLAYGROUND_UI_ROOT").into();
    let listen_host = env::var("PLAYGROUND_UI_LISTEN_HOST").unwrap_or(DEFAULT_LISTEN_HOST.to_string());
    let public_host = env::var("PLAYGROUND_UI_PUBLIC_HOST").unwrap_or_else(|_| listen_host.clone());
    let http_port = env::var("PLAYGROUND_UI_HTTP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_HTTP_PORT);
    let https_port = env::var("PLAYGROUND_UI_HTTPS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_HTTPS_PORT);
    let pkcs12_file = env::var_os("PLAYGROUND_UI_HTTPS_PKCS12_FILE").map(PathBuf::from);
    let pkcs12_password = env::var("PLAYGROUND_UI_HTTPS_PKCS12_PASSWORD").ok();
    let logfile = env::var("PLAYGROUND_LOG_FILE").unwrap_or(DEFAULT_LOG_FILE.to_string());

    let http_address  = (listen_host.as_str(), http_port);
    let https_address = (listen_host.as_str(), https_port);

    let files = Staticfile::new(&root).expect("Unable to open root directory");
    let mut files = Chain::new(files);
    let one_day = Duration::new(ONE_DAY_IN_SECONDS, 0);
    let one_year = Duration::new(ONE_YEAR_IN_SECONDS, 0);

    files.link_after(ModifyWith::new(Cache::new(one_day)));
    files.link_after(Prefix::new(&["assets"], Cache::new(one_year)));
    files.link_after(GuessContentType::new(ContentType::html().0));

    let mut mount = Mount::new();
    mount.mount("/", files);
    mount.mount("/compile", compile);
    mount.mount("/execute", execute);
    mount.mount("/format", format);
    mount.mount("/clippy", clippy);

    let mut chain = Chain::new(mount);
    let file_logger = FileLogger::new(logfile).expect("Unable to create file logger");
    let logger = StatisticLogger::new(file_logger);
    let rewrite = Rewrite::new(vec![vec!["help".into()]], "/index.html".into());

    chain.link_around(logger);
    chain.link_before(rewrite);

    match (pkcs12_file, pkcs12_password) {
        (Some(pkcs12_file), Some(pkcs12_password)) => {
            let redirect = HttpToHttpsRedirect::new(&public_host, https_port).temporary();
            let ssl = NativeTlsServer::new(pkcs12_file, &pkcs12_password)
                .expect("Unable to create the TLS server");

            let _https = Iron::new(chain).https(https_address, ssl)
                .expect("Unable to start HTTPS server");
            info!("Started the HTTPS server on {:?}", https_address);

            info!("Redirecting HTTP traffic to {:?}", redirect);
            let _http = Iron::new(redirect).http(http_address)
                .expect("Unable to start HTTP redirection server");
            info!("Started the HTTP redirection server on {:?}", http_address);
        }
        _ => {
            let _http = Iron::new(chain).http(http_address)
                .expect("Unable to start HTTP server");
            info!("Starting the HTTP server on {:?}", http_address);
        }
    }
}

fn compile(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: CompileRequest| {
        let req = try!(req.try_into());
        sandbox
            .compile(&req)
            .map(CompileResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn execute(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: ExecuteRequest| {
        let req = try!(req.try_into());
        sandbox
            .execute(&req)
            .map(ExecuteResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn format(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: FormatRequest| {
        let req = try!(req.try_into());
        sandbox
            .format(&req)
            .map(FormatResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn clippy(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: ClippyRequest| {
        sandbox
            .clippy(&req.into())
            .map(ClippyResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn with_sandbox<Req, Resp, F>(req: &mut Request, f: F) -> IronResult<Response>
    where F: FnOnce(Sandbox, Req) -> Result<Resp>,
          Req: Deserialize + Clone + Any + 'static,
          Resp: Serialize,
{
    let response = req.get::<bodyparser::Struct<Req>>()
        .map_err(Error::Deserialization)
        .and_then(|r| r.ok_or(Error::RequestMissing))
        .and_then(|req| {
            let sandbox = try!(Sandbox::new());
            let resp = try!(f(sandbox, req));
            let body = try!(serde_json::ser::to_string(&resp));
            Ok(body)
        });

    match response {
        Ok(body) => Ok(Response::with((status::Ok, Header(ContentType::json()), body))),
        Err(err) => {
            let err = ErrorJson { error: err.to_string() };
            match serde_json::ser::to_string(&err) {
                Ok(error_str) => Ok(Response::with((status::InternalServerError, Header(ContentType::json()), error_str))),
                Err(_) => Ok(Response::with((status::InternalServerError, Header(ContentType::json()), FATAL_ERROR_JSON))),
            }
        },
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Sandbox(err: sandbox::Error) {
            description("sandbox operation failed")
            display("Sandbox operation failed: {}", err)
            cause(err)
            from()
        }
        Serialization(err: serde_json::Error) {
            description("unable to serialize response")
            display("Unable to serialize response: {}", err)
            cause(err)
            from()
        }
        Deserialization(err: bodyparser::BodyError) {
            description("unable to deserialize request")
            display("Unable to deserialize request: {}", err)
            cause(err)
            from()
        }
        InvalidTarget(value: String) {
            description("an invalid target was passed")
            display("The value {:?} is not a valid target", value)
        }
        InvalidChannel(value: String) {
            description("an invalid channel was passed")
            display("The value {:?} is not a valid channel", value,)
        }
        InvalidMode(value: String) {
            description("an invalid mode was passed")
            display("The value {:?} is not a valid mode", value)
        }
        RequestMissing {
            description("no request was provided")
            display("No request was provided")
        }
    }
}

type Result<T> = ::std::result::Result<T, Error>;

const FATAL_ERROR_JSON: &'static str =
    r#"{"error": "Multiple cascading errors occurred, abandon all hope"}"#;

#[derive(Debug, Clone, Serialize)]
struct ErrorJson {
    error: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CompileRequest {
    target: String,
    channel: String,
    mode: String,
    #[serde(rename = "crateType")]
    crate_type: String,
    tests: bool,
    code: String,
}

#[derive(Debug, Clone, Serialize)]
struct CompileResponse {
    success: bool,
    code: String,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ExecuteRequest {
    channel: String,
    mode: String,
    #[serde(rename = "crateType")]
    crate_type: String,
    tests: bool,
    code: String,
}

#[derive(Debug, Clone, Serialize)]
struct ExecuteResponse {
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FormatRequest {
    code: String,
    style: String,
}

#[derive(Debug, Clone, Serialize)]
struct FormatResponse {
    success: bool,
    code: String,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ClippyRequest {
    code: String,
}

#[derive(Debug, Clone, Serialize)]
struct ClippyResponse {
    success: bool,
    stdout: String,
    stderr: String,
}

impl TryFrom<CompileRequest> for sandbox::CompileRequest {
    type Error = Error;

    fn try_from(me: CompileRequest) -> Result<Self> {
        Ok(sandbox::CompileRequest {
            target: try!(parse_target(&me.target)),
            channel: try!(parse_channel(&me.channel)),
            mode: try!(parse_mode(&me.mode)),
            crate_type: try!(parse_crate_type(&me.crate_type)),
            tests: me.tests,
            code: me.code,
        })
    }
}

impl From<sandbox::CompileResponse> for CompileResponse {
    fn from(me: sandbox::CompileResponse) -> Self {
        CompileResponse {
            success: me.success,
            code: me.code,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

impl TryFrom<ExecuteRequest> for sandbox::ExecuteRequest {
    type Error = Error;

    fn try_from(me: ExecuteRequest) -> Result<Self> {
        Ok(sandbox::ExecuteRequest {
            channel: try!(parse_channel(&me.channel)),
            mode: try!(parse_mode(&me.mode)),
            crate_type: try!(parse_crate_type(&me.crate_type)),
            tests: me.tests,
            code: me.code,
        })
    }
}

impl From<sandbox::ExecuteResponse> for ExecuteResponse {
    fn from(me: sandbox::ExecuteResponse) -> Self {
        ExecuteResponse {
            success: me.success,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

impl TryFrom<FormatRequest> for sandbox::FormatRequest {
    type Error = Error;

    fn try_from(me: FormatRequest) -> Result<Self> {
        Ok(sandbox::FormatRequest {
            code: me.code,
            style: try!(parse_style(&me.style)),
        })
    }
}

impl From<sandbox::FormatResponse> for FormatResponse {
    fn from(me: sandbox::FormatResponse) -> Self {
        FormatResponse {
            success: me.success,
            code: me.code,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

impl From<ClippyRequest> for sandbox::ClippyRequest {
    fn from(me: ClippyRequest) -> Self {
        sandbox::ClippyRequest {
            code: me.code,
        }
    }
}

impl From<sandbox::ClippyResponse> for ClippyResponse {
    fn from(me: sandbox::ClippyResponse) -> Self {
        ClippyResponse {
            success: me.success,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

fn parse_target(s: &str) -> Result<sandbox::CompileTarget> {
    Ok(match s {
        "asm" => sandbox::CompileTarget::Assembly,
        "llvm-ir" => sandbox::CompileTarget::LlvmIr,
        "mir" => sandbox::CompileTarget::Mir,
        _ => return Err(Error::InvalidTarget(s.into()))
    })
}

fn parse_channel(s: &str) -> Result<sandbox::Channel> {
    Ok(match s {
        "stable" => sandbox::Channel::Stable,
        "beta" => sandbox::Channel::Beta,
        "nightly" => sandbox::Channel::Nightly,
        _ => return Err(Error::InvalidChannel(s.into()))
    })
}

fn parse_mode(s: &str) -> Result<sandbox::Mode> {
    Ok(match s {
        "debug" => sandbox::Mode::Debug,
        "release" => sandbox::Mode::Release,
        _ => return Err(Error::InvalidMode(s.into()))
    })
}

fn parse_crate_type(s: &str) -> Result<sandbox::CrateType> {
    Ok(match s {
        "bin" => sandbox::CrateType::Binary,
        _ => sandbox::CrateType::Library,
    })
}

fn parse_style(s: &str) -> Result<sandbox::FormatStyle> {
    Ok(match s {
        "rfc" => sandbox::FormatStyle::Rfc,
        _ => sandbox::FormatStyle::Default,
    })
}
