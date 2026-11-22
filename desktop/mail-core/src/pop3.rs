//! Minimal async POP3 client (RFC 1939) over TCP / TLS (`async_native_tls`).

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pop3Config {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pop3Stat {
    pub count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pop3Uidl {
    pub seq: usize,
    pub uid: String,
}

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

pub struct Pop3Client {
    reader: BufReader<Box<dyn AsyncReadWrite>>,
}

impl Pop3Client {
    pub async fn connect(cfg: &Pop3Config) -> Result<Self, String> {
        let addr = format!("{}:{}", cfg.host, cfg.port);
        let tcp = TcpStream::connect(&addr)
            .await
            .map_err(|e| format!("POP3 连接失败 ({addr}): {e}"))?;
        let stream: Box<dyn AsyncReadWrite> = if cfg.tls {
            let connector = async_native_tls::TlsConnector::new();
            let tls = connector
                .connect(&cfg.host, tcp)
                .await
                .map_err(|e| format!("POP3 TLS 握手失败: {e}"))?;
            Box::new(tls)
        } else {
            Box::new(tcp)
        };
        let mut client = Self {
            reader: BufReader::new(stream),
        };
        let greeting = client.read_status_line().await?;
        if !greeting.starts_with("+OK") {
            return Err(format!("POP3 握手异常: {greeting}"));
        }
        client.login(&cfg.username, &cfg.password).await?;
        Ok(client)
    }

    async fn read_status_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("POP3 读取失败: {e}"))?;
        if n == 0 {
            return Err("POP3 连接已关闭".to_string());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    async fn send_cmd(&mut self, cmd: &str) -> Result<String, String> {
        let wire = format!("{cmd}\r\n");
        self.reader
            .get_mut()
            .write_all(wire.as_bytes())
            .await
            .map_err(|e| format!("POP3 发送失败: {e}"))?;
        self.reader
            .get_mut()
            .flush()
            .await
            .map_err(|e| format!("POP3 flush 失败: {e}"))?;
        let line = self.read_status_line().await?;
        if !line.starts_with("+OK") {
            return Err(format!("POP3 命令失败 ({cmd}): {line}"));
        }
        Ok(line)
    }

    async fn read_multiline(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            let mut line = String::new();
            let n = self
                .reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("POP3 读取多行失败: {e}"))?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed == "." {
                break;
            }
            let unstuffed = trimmed.strip_prefix("..").unwrap_or(trimmed);
            out.extend_from_slice(unstuffed.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        Ok(out)
    }

    async fn login(&mut self, user: &str, pass: &str) -> Result<(), String> {
        self.send_cmd(&format!("USER {user}")).await?;
        self.send_cmd(&format!("PASS {pass}")).await?;
        Ok(())
    }

    pub async fn stat(&mut self) -> Result<Pop3Stat, String> {
        let line = self.send_cmd("STAT").await?;
        parse_stat_line(&line)
    }

    pub async fn uidl(&mut self) -> Result<Vec<Pop3Uidl>, String> {
        self.send_cmd("UIDL").await?;
        let raw = self.read_multiline().await?;
        let text = String::from_utf8_lossy(&raw);
        let mut out = Vec::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(seq_s), Some(uid)) = (parts.next(), parts.next()) {
                if let Ok(seq) = seq_s.parse::<usize>() {
                    out.push(Pop3Uidl {
                        seq,
                        uid: uid.to_string(),
                    });
                }
            }
        }
        Ok(out)
    }

    pub async fn retr(&mut self, seq: usize) -> Result<Vec<u8>, String> {
        self.send_cmd(&format!("RETR {seq}")).await?;
        self.read_multiline().await
    }

    pub async fn dele(&mut self, seq: usize) -> Result<(), String> {
        self.send_cmd(&format!("DELE {seq}")).await?;
        Ok(())
    }

    pub async fn quit(&mut self) -> Result<(), String> {
        self.send_cmd("QUIT").await?;
        Ok(())
    }
}

pub fn parse_stat_line(line: &str) -> Result<Pop3Stat, String> {
    let mut parts = line.split_whitespace();
    let status = parts.next().unwrap_or_default();
    if !status.starts_with("+OK") {
        return Err(format!("无效 STAT 响应: {line}"));
    }
    let count = parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| format!("STAT 缺少邮件数量: {line}"))?;
    let total_bytes = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(Pop3Stat { count, total_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pop3_stat() {
        let stat = parse_stat_line("+OK 12 4096").unwrap();
        assert_eq!(stat.count, 12);
        assert_eq!(stat.total_bytes, 4096);
        assert!(parse_stat_line("-ERR invalid").is_err());
    }
}
