//! 解析瀏覽器「Copy as cURL (bash)」得到的指令，取出 URL 與請求頭。

use anyhow::{bail, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// 這些頭由 HTTP 客戶端自己處理，帶上反而可能出錯（例如 gzip 後的 FLV）。
const DROP_HEADERS: [&str; 5] = ["accept-encoding", "content-length", "host", "range", "connection"];

/// 依 POSIX shell 規則切詞：支援 '...'、"..."、$'...'、反斜線與行尾續行。
fn split(cmd: &str) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                Some(n) => {
                    cur.push(n);
                    in_word = true;
                }
                None => {}
            },
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(n) => cur.push(n),
                        None => bail!("單引號沒有閉合"),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                            Some('\n') => {}
                            Some(n) => {
                                cur.push('\\');
                                cur.push(n);
                            }
                            None => bail!("雙引號沒有閉合"),
                        },
                        Some(n) => cur.push(n),
                        None => bail!("雙引號沒有閉合"),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some('r') => cur.push('\r'),
                            Some('x') => {
                                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                    cur.push(b as char);
                                }
                            }
                            Some(n) => cur.push(n),
                            None => bail!("$'...' 沒有閉合"),
                        },
                        Some(n) => cur.push(n),
                        None => bail!("$'...' 沒有閉合"),
                    }
                }
            }
            _ => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

pub fn parse(cmd: &str) -> Result<Request> {
    let words = split(cmd.trim())?;
    let mut it = words.into_iter().peekable();
    if it.peek().map(String::as_str) == Some("curl") {
        it.next();
    }
    let mut req = Request::default();
    let add = |req: &mut Request, name: &str, value: &str| {
        if !DROP_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            req.headers.push((name.to_string(), value.trim().to_string()));
        }
    };
    while let Some(w) = it.next() {
        match w.as_str() {
            "-H" | "--header" => {
                if let Some(h) = it.next() {
                    if let Some((k, v)) = h.split_once(':') {
                        add(&mut req, k.trim(), v);
                    }
                }
            }
            "-b" | "--cookie" => {
                if let Some(v) = it.next() {
                    add(&mut req, "Cookie", &v);
                }
            }
            "-A" | "--user-agent" => {
                if let Some(v) = it.next() {
                    add(&mut req, "User-Agent", &v);
                }
            }
            "-e" | "--referer" => {
                if let Some(v) = it.next() {
                    add(&mut req, "Referer", &v);
                }
            }
            "--url" => {
                if let Some(v) = it.next() {
                    req.url = v;
                }
            }
            // 帶參數但與拉流無關的選項
            "-X" | "--request" | "-d" | "--data" | "--data-raw" | "--data-binary" | "-o" | "--output" | "-u"
            | "--user" | "-x" | "--proxy" | "-m" | "--max-time" | "--connect-timeout" => {
                it.next();
            }
            s if s.starts_with("http://") || s.starts_with("https://") => req.url = w,
            _ => {}
        }
    }
    if req.url.is_empty() {
        bail!("cURL 指令裡找不到 http(s) 網址");
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_bash_copy() {
        let cmd = r#"curl 'https://pull-flv-l26.douyincdn.com/stage/stream-123_or4.flv?expire=1&sign=abc' \
  -H 'accept: */*' \
  -H 'accept-encoding: gzip' \
  -b 'sessionid=xyz; ttwid=1%7Cabc' \
  -H $'referer: https://live.douyin.com/12345' \
  -H "user-agent: Mozilla/5.0 \"test\"" \
  --compressed"#;
        let r = parse(cmd).unwrap();
        assert_eq!(r.url, "https://pull-flv-l26.douyincdn.com/stage/stream-123_or4.flv?expire=1&sign=abc");
        assert_eq!(
            r.headers,
            vec![
                ("accept".into(), "*/*".into()),
                ("Cookie".into(), "sessionid=xyz; ttwid=1%7Cabc".into()),
                ("referer".into(), "https://live.douyin.com/12345".into()),
                ("user-agent".into(), "Mozilla/5.0 \"test\"".into()),
            ]
        );
    }

    #[test]
    fn plain_url_without_curl() {
        assert_eq!(parse("https://a.b/c.flv").unwrap().url, "https://a.b/c.flv");
        assert!(parse("curl -H 'a: b'").is_err());
    }
}
