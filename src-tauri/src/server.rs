//! Проверка статуса игрового MC-сервера через Server List Ping (SLP).
//!
//! Лаунчер по TCP делает handshake + status request (как клиент в списке серверов)
//! и парсит JSON-ответ: онлайн ли сервер и сколько игроков сейчас. Нужен для
//! подписи «Онлайн/Оффлайн» и смены кнопки «Играть» → «Играть оффлайн», когда
//! сервер недоступен. Любая ошибка сети/таймаут → `online: false`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config;

/// Версия протокола 1.21.1 (для handshake; на статус-пинг сервер отвечает при
/// любом значении, но шлём корректный).
const PROTOCOL_1_21_1: i32 = 767;
/// Таймаут на весь пинг (connect + обмен).
const PING_TIMEOUT: Duration = Duration::from_secs(3);
/// Лимит на размер JSON-ответа (защита от мусора; favicon помещается).
const MAX_RESPONSE: usize = 64 * 1024;

/// Статус сервера для фронтенда.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerStatus {
    /// Сервер ответил на пинг.
    pub online: bool,
    /// Игроков сейчас (0, если оффлайн).
    pub players_online: u32,
    /// Лимит игроков (0, если неизвестно).
    pub players_max: u32,
}

impl ServerStatus {
    fn offline() -> Self {
        Self::default()
    }
}

/// Запросить статус MC-сервера из [`config::SERVER_ADDR`]. Никогда не возвращает
/// ошибку — при недоступности/таймауте отдаёт `online: false`.
pub async fn status() -> ServerStatus {
    let (host, port) = config::server_host_port();
    match tokio::time::timeout(PING_TIMEOUT, ping(&host, port)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            log::info!("server_status: сервер недоступен ({host}:{port}): {e}");
            ServerStatus::offline()
        }
        Err(_) => {
            log::info!("server_status: таймаут пинга {host}:{port}");
            ServerStatus::offline()
        }
    }
}

/// Один SLP-обмен: handshake(next=1) → status request → парс JSON.
async fn ping(host: &str, port: u16) -> std::io::Result<ServerStatus> {
    let mut stream = TcpStream::connect((host, port)).await?;

    // --- C→S Handshake (state=1) ---
    let mut hs = Vec::new();
    write_varint(&mut hs, 0x00); // packet id
    write_varint(&mut hs, PROTOCOL_1_21_1);
    write_string(&mut hs, host);
    hs.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut hs, 1); // next state = status
    write_frame(&mut stream, &hs).await?;

    // --- C→S Status request (empty) ---
    let mut req = Vec::new();
    write_varint(&mut req, 0x00);
    write_frame(&mut stream, &req).await?;

    // --- S→C Status response ---
    let len = read_varint(&mut stream).await?;
    if len <= 0 || len as usize > MAX_RESPONSE {
        return Err(err("некорректная длина ответа"));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;

    // packet id (varint) + json string (varint len + bytes)
    let mut cur = &buf[..];
    let _packet_id = take_varint(&mut cur)?;
    let json_len = take_varint(&mut cur)? as usize;
    if json_len > cur.len() {
        return Err(err("обрезанный JSON статуса"));
    }
    let json = &cur[..json_len];

    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|_| err("невалидный JSON статуса"))?;
    Ok(ServerStatus {
        online: true,
        players_online: v["players"]["online"].as_u64().unwrap_or(0) as u32,
        players_max: v["players"]["max"].as_u64().unwrap_or(0) as u32,
    })
}

fn err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

/// Отправить пакет с префиксом длины (VarInt).
async fn write_frame(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    let mut framed = Vec::new();
    write_varint(&mut framed, body.len() as i32);
    framed.extend_from_slice(body);
    stream.write_all(&framed).await?;
    stream.flush().await
}

fn write_varint(buf: &mut Vec<u8>, value: i32) {
    let mut val = value as u32;
    loop {
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        if val != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if val == 0 {
            break;
        }
    }
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_varint(buf, s.len() as i32);
    buf.extend_from_slice(s.as_bytes());
}

/// Прочитать VarInt из потока (до 5 байт).
async fn read_varint(stream: &mut TcpStream) -> std::io::Result<i32> {
    let mut result: i32 = 0;
    let mut shift = 0;
    loop {
        let byte = stream.read_u8().await?;
        result |= ((byte & 0x7F) as i32) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 35 {
            return Err(err("VarInt слишком длинный"));
        }
    }
    Ok(result)
}

/// Прочитать VarInt из среза, сдвигая его.
fn take_varint(buf: &mut &[u8]) -> std::io::Result<i32> {
    let mut result: i32 = 0;
    let mut shift = 0;
    loop {
        let (&byte, rest) = buf.split_first().ok_or_else(|| err("VarInt: конец данных"))?;
        *buf = rest;
        result |= ((byte & 0x7F) as i32) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 35 {
            return Err(err("VarInt слишком длинный"));
        }
    }
    Ok(result)
}
