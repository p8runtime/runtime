//! curl_getinfo(): what the last transfer left in the handle.

use crate::{State, c};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Ctx;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

/// The session info libcurl keeps, in its own units (microseconds, bytes).
#[derive(Clone)]
pub(crate) struct Info {
    pub url: String,
    pub content_type: Option<String>,
    pub http_code: i64,
    pub header_size: i64,
    pub request_size: i64,
    pub redirect_count: i64,
    pub total_us: i64,
    pub namelookup_us: i64,
    pub connect_us: i64,
    pub appconnect_us: i64,
    pub pretransfer_us: i64,
    pub starttransfer_us: i64,
    pub redirect_us: i64,
    pub size_upload: i64,
    pub size_download: i64,
    pub download_content_length: i64,
    pub upload_content_length: i64,
    pub redirect_url: String,
    pub primary_ip: String,
    pub primary_port: i64,
    pub local_ip: String,
    pub local_port: i64,
    pub http_version: i64,
    pub protocol: i64,
    pub scheme: String,
    pub request_header: Option<String>,
    pub effective_method: String,
}

impl Default for Info {
    fn default() -> Info {
        Info {
            url: String::new(),
            content_type: None,
            http_code: 0,
            header_size: 0,
            request_size: 0,
            redirect_count: 0,
            total_us: 0,
            namelookup_us: 0,
            connect_us: 0,
            appconnect_us: 0,
            pretransfer_us: 0,
            starttransfer_us: 0,
            redirect_us: 0,
            size_upload: 0,
            size_download: 0,
            download_content_length: -1,
            upload_content_length: -1,
            redirect_url: String::new(),
            primary_ip: String::new(),
            primary_port: -1,
            local_ip: String::new(),
            local_port: -1,
            http_version: 0,
            protocol: 0,
            scheme: String::new(),
            request_header: None,
            effective_method: "GET".into(),
        }
    }
}

const SYSTEM_CAPATH: &str = "/etc/ssl/certs";
const SYSTEM_CAINFO: &str = "/etc/ssl/certs/ca-certificates.crt";

fn secs(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

fn per_second(bytes: i64, total_us: i64) -> f64 {
    if total_us <= 0 { 0.0 } else { (bytes as f64 / secs(total_us)).floor() }
}

fn opt_string(st: &State, option: i64) -> Option<String> {
    st.opts.get(&option).map(|v| crate::lossy(&crate::bytes_of(v)))
}

fn url_of(st: &State) -> String {
    if st.info.url.is_empty() { opt_string(st, c::CURLOPT_URL).unwrap_or_default() } else { st.info.url.clone() }
}

fn put(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(Key::from_string(PhpStr::from(k)), v);
}

fn s(v: &str) -> Value {
    Value::str(v)
}

/// curl_getinfo($ch): every key, in PHP's order.
pub(crate) fn all(st: &State) -> Value {
    let i = &st.info;
    let mut a = PhpArray::new();
    put(&mut a, "url", s(&url_of(st)));
    put(&mut a, "content_type", i.content_type.as_deref().map_or(Value::NULL, s));
    put(&mut a, "http_code", Value::int(i.http_code));
    put(&mut a, "header_size", Value::int(i.header_size));
    put(&mut a, "request_size", Value::int(i.request_size));
    put(&mut a, "filetime", Value::int(-1));
    put(&mut a, "ssl_verify_result", Value::int(0));
    put(&mut a, "redirect_count", Value::int(i.redirect_count));
    put(&mut a, "total_time", Value::float(secs(i.total_us)));
    put(&mut a, "namelookup_time", Value::float(secs(i.namelookup_us)));
    put(&mut a, "connect_time", Value::float(secs(i.connect_us)));
    put(&mut a, "pretransfer_time", Value::float(secs(i.pretransfer_us)));
    put(&mut a, "size_upload", Value::float(i.size_upload as f64));
    put(&mut a, "size_download", Value::float(i.size_download as f64));
    put(&mut a, "speed_download", Value::float(per_second(i.size_download, i.total_us)));
    put(&mut a, "speed_upload", Value::float(per_second(i.size_upload, i.total_us)));
    put(&mut a, "download_content_length", Value::float(i.download_content_length as f64));
    put(&mut a, "upload_content_length", Value::float(i.upload_content_length as f64));
    put(&mut a, "starttransfer_time", Value::float(secs(i.starttransfer_us)));
    put(&mut a, "redirect_time", Value::float(secs(i.redirect_us)));
    put(&mut a, "redirect_url", s(&i.redirect_url));
    put(&mut a, "primary_ip", s(&i.primary_ip));
    put(&mut a, "certinfo", Value::array(PhpArray::new()));
    put(&mut a, "primary_port", Value::int(i.primary_port));
    put(&mut a, "local_ip", s(&i.local_ip));
    put(&mut a, "local_port", Value::int(i.local_port));
    put(&mut a, "http_version", Value::int(i.http_version));
    put(&mut a, "protocol", Value::int(i.protocol));
    put(&mut a, "ssl_verifyresult", Value::int(0));
    put(&mut a, "scheme", s(&i.scheme));
    put(&mut a, "appconnect_time_us", Value::int(i.appconnect_us));
    put(&mut a, "connect_time_us", Value::int(i.connect_us));
    put(&mut a, "namelookup_time_us", Value::int(i.namelookup_us));
    put(&mut a, "pretransfer_time_us", Value::int(i.pretransfer_us));
    put(&mut a, "redirect_time_us", Value::int(i.redirect_us));
    put(&mut a, "starttransfer_time_us", Value::int(i.starttransfer_us));
    put(&mut a, "posttransfer_time_us", Value::int(i.pretransfer_us));
    put(&mut a, "total_time_us", Value::int(i.total_us));
    if let Some(h) = &i.request_header {
        put(&mut a, "request_header", s(h));
    }
    put(&mut a, "effective_method", s(&i.effective_method));
    put(&mut a, "capath", s(&opt_string(st, c::CURLOPT_CAPATH).unwrap_or_else(|| SYSTEM_CAPATH.into())));
    put(&mut a, "cainfo", s(&opt_string(st, c::CURLOPT_CAINFO).unwrap_or_else(|| SYSTEM_CAINFO.into())));
    Value::array(a)
}

const TYPE_MASK: i64 = 0xf0_0000;
const TYPE_STRING: i64 = 0x10_0000;
const TYPE_LONG: i64 = 0x20_0000;
const TYPE_DOUBLE: i64 = 0x30_0000;
const TYPE_SLIST: i64 = 0x40_0000;
const TYPE_OFF_T: i64 = 0x60_0000;

/// curl_getinfo($ch, $option).
pub(crate) fn one(_: &mut dyn Ctx, st: &State, option: i64) -> Value {
    let i = &st.info;
    match option {
        c::CURLINFO_HEADER_OUT => return i.request_header.as_deref().map_or(Value::FALSE, s),
        c::CURLINFO_CERTINFO => return Value::array(PhpArray::new()),
        c::CURLINFO_PRIVATE => return st.private.clone().unwrap_or(Value::FALSE),
        c::CURLINFO_COOKIELIST => return Value::array(PhpArray::from_list(crate::cookie::list(st).iter().map(|l| Value::str(l.as_str())).collect())),
        c::CURLINFO_EFFECTIVE_URL => return s(&url_of(st)),
        c::CURLINFO_CONTENT_TYPE => return i.content_type.as_deref().map_or(Value::FALSE, s),
        c::CURLINFO_REDIRECT_URL => return if i.redirect_url.is_empty() { Value::FALSE } else { s(&i.redirect_url) },
        c::CURLINFO_PRIMARY_IP => return s(&i.primary_ip),
        c::CURLINFO_LOCAL_IP => return s(&i.local_ip),
        c::CURLINFO_SCHEME => return if i.scheme.is_empty() { Value::FALSE } else { s(&i.scheme) },
        c::CURLINFO_EFFECTIVE_METHOD => return s(&i.effective_method),
        c::CURLINFO_CAINFO => return s(&opt_string(st, c::CURLOPT_CAINFO).unwrap_or_else(|| SYSTEM_CAINFO.into())),
        c::CURLINFO_CAPATH => return s(&opt_string(st, c::CURLOPT_CAPATH).unwrap_or_else(|| SYSTEM_CAPATH.into())),
        c::CURLINFO_RESPONSE_CODE => return Value::int(i.http_code),
        c::CURLINFO_HEADER_SIZE => return Value::int(i.header_size),
        c::CURLINFO_REQUEST_SIZE => return Value::int(i.request_size),
        c::CURLINFO_FILETIME => return Value::int(-1),
        c::CURLINFO_REDIRECT_COUNT => return Value::int(i.redirect_count),
        c::CURLINFO_PRIMARY_PORT => return Value::int(i.primary_port),
        c::CURLINFO_LOCAL_PORT => return Value::int(i.local_port),
        c::CURLINFO_HTTP_VERSION => return Value::int(i.http_version),
        c::CURLINFO_PROTOCOL => return Value::int(i.protocol),
        c::CURLINFO_TOTAL_TIME => return Value::float(secs(i.total_us)),
        c::CURLINFO_NAMELOOKUP_TIME => return Value::float(secs(i.namelookup_us)),
        c::CURLINFO_CONNECT_TIME => return Value::float(secs(i.connect_us)),
        c::CURLINFO_APPCONNECT_TIME => return Value::float(secs(i.appconnect_us)),
        c::CURLINFO_PRETRANSFER_TIME => return Value::float(secs(i.pretransfer_us)),
        c::CURLINFO_STARTTRANSFER_TIME => return Value::float(secs(i.starttransfer_us)),
        c::CURLINFO_REDIRECT_TIME => return Value::float(secs(i.redirect_us)),
        c::CURLINFO_SIZE_UPLOAD => return Value::float(i.size_upload as f64),
        c::CURLINFO_SIZE_DOWNLOAD => return Value::float(i.size_download as f64),
        c::CURLINFO_SPEED_DOWNLOAD => return Value::float(per_second(i.size_download, i.total_us)),
        c::CURLINFO_SPEED_UPLOAD => return Value::float(per_second(i.size_upload, i.total_us)),
        c::CURLINFO_CONTENT_LENGTH_DOWNLOAD => return Value::float(i.download_content_length as f64),
        c::CURLINFO_CONTENT_LENGTH_UPLOAD => return Value::float(i.upload_content_length as f64),
        c::CURLINFO_TOTAL_TIME_T => return Value::int(i.total_us),
        c::CURLINFO_NAMELOOKUP_TIME_T => return Value::int(i.namelookup_us),
        c::CURLINFO_CONNECT_TIME_T => return Value::int(i.connect_us),
        c::CURLINFO_APPCONNECT_TIME_T => return Value::int(i.appconnect_us),
        c::CURLINFO_PRETRANSFER_TIME_T => return Value::int(i.pretransfer_us),
        c::CURLINFO_STARTTRANSFER_TIME_T => return Value::int(i.starttransfer_us),
        c::CURLINFO_REDIRECT_TIME_T => return Value::int(i.redirect_us),
        c::CURLINFO_POSTTRANSFER_TIME_T => return Value::int(i.pretransfer_us),
        c::CURLINFO_SIZE_UPLOAD_T => return Value::int(i.size_upload),
        c::CURLINFO_SIZE_DOWNLOAD_T => return Value::int(i.size_download),
        c::CURLINFO_SPEED_DOWNLOAD_T => return Value::int(per_second(i.size_download, i.total_us) as i64),
        c::CURLINFO_SPEED_UPLOAD_T => return Value::int(per_second(i.size_upload, i.total_us) as i64),
        c::CURLINFO_CONTENT_LENGTH_DOWNLOAD_T => return Value::int(i.download_content_length),
        c::CURLINFO_CONTENT_LENGTH_UPLOAD_T => return Value::int(i.upload_content_length),
        _ => {}
    }
    // What libcurl gives for the rest of its info, by the type in the number.
    match option & TYPE_MASK {
        TYPE_STRING => Value::FALSE,
        TYPE_LONG | TYPE_OFF_T => Value::int(0),
        TYPE_DOUBLE => Value::float(0.0),
        TYPE_SLIST => Value::array(PhpArray::new()),
        _ => Value::FALSE,
    }
}
