//! libpq's connection strings: `host=localhost port=5432 dbname='my db'`,
//! as conninfo_parse() reads them, and the defaults it fills in (the PG*
//! environment variables libpq reads, then its built-in ones).
//!
//! Reference: postgresql/src/interfaces/libpq/fe-connect.c (PQconninfoOptions,
//! conninfo_parse, conninfo_add_defaults)

/// The keywords libpq 17 knows. Others are an error.
const KEYWORDS: &[&str] = &[
    "service",
    "user",
    "password",
    "passfile",
    "channel_binding",
    "connect_timeout",
    "dbname",
    "host",
    "hostaddr",
    "port",
    "client_encoding",
    "options",
    "application_name",
    "fallback_application_name",
    "keepalives",
    "keepalives_idle",
    "keepalives_interval",
    "keepalives_count",
    "tcp_user_timeout",
    "sslmode",
    "sslnegotiation",
    "sslcompression",
    "sslcert",
    "sslkey",
    "sslcertmode",
    "sslpassword",
    "sslrootcert",
    "sslcrl",
    "sslcrldir",
    "sslsni",
    "requirepeer",
    "require_auth",
    "ssl_min_protocol_version",
    "ssl_max_protocol_version",
    "gssencmode",
    "krbsrvname",
    "gsslib",
    "gssdelegation",
    "replication",
    "target_session_attrs",
    "load_balance_hosts",
];

/// The environment variable behind a keyword's default.
const ENV: &[(&str, &str)] = &[
    ("host", "PGHOST"),
    ("hostaddr", "PGHOSTADDR"),
    ("port", "PGPORT"),
    ("dbname", "PGDATABASE"),
    ("user", "PGUSER"),
    ("password", "PGPASSWORD"),
    ("options", "PGOPTIONS"),
    ("application_name", "PGAPPNAME"),
    ("connect_timeout", "PGCONNECT_TIMEOUT"),
    ("sslmode", "PGSSLMODE"),
    ("client_encoding", "PGCLIENTENCODING"),
];

const SSLMODES: &[&str] = &["disable", "allow", "prefer", "require", "verify-ca", "verify-full"];

/// The parsed options, in the order given (a later one wins).
#[derive(Debug, Default)]
pub struct Params {
    entries: Vec<(String, String)>,
}

impl Params {
    pub fn get(&self, key: &str) -> Option<&str> {
        if let Some((_, v)) = self.entries.iter().rev().find(|(k, _)| k == key) {
            return Some(v.as_str());
        }
        None
    }

    /// The user to log in as: given, PGUSER, or the OS user.
    pub fn user(&self) -> String {
        self.get("user").filter(|u| !u.is_empty()).map(str::to_string).unwrap_or_else(|| std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "postgres".into()))
    }

    /// The (host, port) pairs to try: a host may be a list, and a path is
    /// the directory of a Unix socket (the default: /tmp).
    pub fn hosts(&self) -> Vec<(String, String)> {
        let host = self.get("host").filter(|h| !h.is_empty()).or_else(|| self.get("hostaddr").filter(|h| !h.is_empty())).unwrap_or("/tmp");
        let hosts: Vec<String> = host.split(',').map(|h| if h.is_empty() { "/tmp".to_string() } else { h.to_string() }).collect();
        let port = self.get("port").filter(|p| !p.is_empty()).unwrap_or("5432");
        let ports: Vec<&str> = port.split(',').collect();
        hosts.iter().enumerate().map(|(i, h)| (h.clone(), ports.get(i).or(ports.first()).map_or("5432", |p| if p.is_empty() { "5432" } else { p }).to_string())).collect()
    }
}

/// conninfo_parse() and the defaults; Err is libpq's message.
pub fn parse(s: &str) -> Result<Params, String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut entries: Vec<(String, String)> = Vec::new();
    let space = |c: u8| c.is_ascii_whitespace();
    loop {
        while i < b.len() && space(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && !space(b[i]) {
            i += 1;
        }
        let name = String::from_utf8_lossy(&b[start..i]).into_owned();
        while i < b.len() && space(b[i]) {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            return Err(format!("missing \"=\" after \"{name}\" in connection info string\n"));
        }
        i += 1;
        while i < b.len() && space(b[i]) {
            i += 1;
        }
        let mut value = Vec::new();
        if i < b.len() && b[i] == b'\'' {
            i += 1;
            loop {
                if i >= b.len() {
                    return Err("unterminated quoted string in connection info string\n".into());
                }
                match b[i] {
                    b'\\' => {
                        i += 1;
                        if i < b.len() {
                            value.push(b[i]);
                            i += 1;
                        }
                    }
                    b'\'' => {
                        i += 1;
                        break;
                    }
                    c => {
                        value.push(c);
                        i += 1;
                    }
                }
            }
        } else {
            while i < b.len() && !space(b[i]) {
                if b[i] == b'\\' {
                    i += 1;
                    if i < b.len() {
                        value.push(b[i]);
                        i += 1;
                    }
                } else {
                    value.push(b[i]);
                    i += 1;
                }
            }
        }
        if !KEYWORDS.contains(&name.as_str()) {
            return Err(format!("invalid connection option \"{name}\"\n"));
        }
        entries.push((name, String::from_utf8_lossy(&value).into_owned()));
    }
    let mut p = Params { entries };
    for (key, var) in ENV {
        if p.get(key).is_none()
            && let Ok(v) = std::env::var(var)
        {
            p.entries.push((key.to_string(), v));
        }
    }
    if let Some(mode) = p.get("sslmode")
        && !SSLMODES.contains(&mode)
    {
        return Err(format!("invalid sslmode value: \"{mode}\"\n"));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_and_escaped_values() {
        let p = parse("host=db port = 6543 dbname='my \\'db' user=a\\ b").expect("parses");
        assert_eq!(p.get("host"), Some("db"));
        assert_eq!(p.get("dbname"), Some("my 'db"));
        assert_eq!(p.get("user"), Some("a b"));
        assert_eq!(p.hosts(), vec![("db".to_string(), "6543".to_string())]);
    }

    #[test]
    fn libpq_errors() {
        assert_eq!(parse("host").unwrap_err(), "missing \"=\" after \"host\" in connection info string\n");
        assert_eq!(parse("nope=1").unwrap_err(), "invalid connection option \"nope\"\n");
        assert_eq!(parse("dbname='x").unwrap_err(), "unterminated quoted string in connection info string\n");
    }
}
