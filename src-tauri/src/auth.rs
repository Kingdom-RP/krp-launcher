//! Авторизация через drasl (Yggdrasil-совместимый сервер) — фаза 6.
//!
//! Два потока против одного и того же сервера:
//! - **drasl REST** (`/drasl/api/v2`): регистрация и управление скином (Bearer
//!   `apiToken`).
//! - **Yggdrasil** (`/authlib-injector/authserver`): игровой `accessToken` для
//!   запуска Minecraft (вместе с `-javaagent:authlib-injector.jar=<base>/authlib-injector`).
//!
//! Оба входа — по одному логину/паролю. Храним токены (НЕ пароль) в
//! `settings.json`; на запуске сессию валидируем/обновляем.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{LauncherError, Result};

/// Сохраняемый аккаунт игрока (в `settings.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Логин в drasl.
    pub username: String,
    /// Игровой ник (имя профиля).
    pub player_name: String,
    /// UUID игрока в drasl (с дефисами) — для skin-API.
    pub player_uuid: String,
    /// UUID для запуска MC (без дефисов, как `selectedProfile.id`).
    pub mc_uuid: String,
    /// Токен drasl REST (операции со скином).
    pub api_token: String,
    /// Игровой Yggdrasil-токен.
    pub access_token: String,
    /// Клиентский Yggdrasil-токен (для validate/refresh).
    pub client_token: String,
}

/// Публичная инфа об аккаунте для фронтенда (без секретов).
#[derive(Debug, Clone, Serialize)]
pub struct AccountInfo {
    pub username: String,
    pub player_name: String,
    pub uuid: String,
    /// URL текущего скина (может быть пустым).
    pub skin_url: Option<String>,
}

fn ygg_url(base: &str) -> String {
    format!("{}/authlib-injector/authserver", base.trim_end_matches('/'))
}

fn rest_url(base: &str) -> String {
    format!("{}/drasl/api/v2", base.trim_end_matches('/'))
}

/// Достать понятное сообщение об ошибке из ответа drasl/Yggdrasil.
async fn api_error(resp: reqwest::Response) -> LauncherError {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    // drasl REST: {"message": "..."}; Yggdrasil: {"errorMessage": "..."}.
    let msg = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("errorMessage"))
                .and_then(|m| m.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("HTTP {status}"));
    LauncherError::Other(msg)
}

/// Yggdrasil `authenticate` → (accessToken, clientToken, mc_uuid, name).
/// `clientToken` не передаём — drasl сгенерит и вернёт свой.
async fn ygg_authenticate(
    client: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
) -> Result<(String, String, String, String)> {
    let body = json!({
        "agent": { "name": "Minecraft", "version": 1 },
        "username": username,
        "password": password,
    });
    let resp = client
        .post(format!("{}/authenticate", ygg_url(base)))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let v: serde_json::Value = resp.json().await?;
    let access = str_field(&v, "accessToken")?;
    let ctoken = str_field(&v, "clientToken")?;
    let id = v["selectedProfile"]["id"]
        .as_str()
        .ok_or_else(|| LauncherError::Other("нет selectedProfile.id".into()))?
        .to_owned();
    let name = v["selectedProfile"]["name"].as_str().unwrap_or("").to_owned();
    Ok((access, ctoken, id, name))
}

fn str_field(v: &serde_json::Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| LauncherError::Other(format!("в ответе нет поля {key}")))
}

/// Собрать `Account` из ответа drasl (user с players) + игровой сессии.
async fn account_from(
    client: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
    api_token: String,
    user: &serde_json::Value,
) -> Result<Account> {
    let player = user["players"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| LauncherError::Other("у аккаунта нет игрока".into()))?;
    let player_uuid = str_field(player, "uuid")?;

    let (access_token, client_token, mc_uuid, name) =
        ygg_authenticate(client, base, username, password).await?;

    Ok(Account {
        username: username.to_owned(),
        player_name: if name.is_empty() {
            player["name"].as_str().unwrap_or(username).to_owned()
        } else {
            name
        },
        player_uuid,
        mc_uuid,
        api_token,
        access_token,
        client_token,
    })
}

/// Регистрация нового аккаунта (логин+пароль). Ник = логину.
pub async fn register(
    client: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
) -> Result<Account> {
    let body = json!({
        "username": username,
        "password": password,
        "playerName": username,
        "requestApiToken": true,
    });
    let resp = client
        .post(format!("{}/users", rest_url(base)))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let v: serde_json::Value = resp.json().await?;
    let api_token = str_field(&v, "apiToken")?;
    account_from(client, base, username, password, api_token, &v["user"]).await
}

/// Вход существующего аккаунта.
pub async fn login(
    client: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
) -> Result<Account> {
    let body = json!({ "username": username, "password": password });
    let resp = client
        .post(format!("{}/login", rest_url(base)))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let v: serde_json::Value = resp.json().await?;
    let api_token = str_field(&v, "apiToken")?;
    account_from(client, base, username, password, api_token, &v["user"]).await
}

/// Проверить игровой токен (Yggdrasil `validate`): `true` — валиден.
async fn ygg_validate(client: &reqwest::Client, base: &str, account: &Account) -> Result<bool> {
    let body = json!({ "accessToken": account.access_token, "clientToken": account.client_token });
    let resp = client
        .post(format!("{}/validate", ygg_url(base)))
        .json(&body)
        .send()
        .await?;
    // 204 No Content — валиден; 403 — протух.
    Ok(resp.status().is_success())
}

/// Обновить игровой токен (Yggdrasil `refresh`).
async fn ygg_refresh(client: &reqwest::Client, base: &str, account: &mut Account) -> Result<()> {
    let body = json!({ "accessToken": account.access_token, "clientToken": account.client_token });
    let resp = client
        .post(format!("{}/refresh", ygg_url(base)))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    let v: serde_json::Value = resp.json().await?;
    account.access_token = str_field(&v, "accessToken")?;
    account.client_token = str_field(&v, "clientToken")?;
    Ok(())
}

/// Убедиться, что игровая сессия валидна (перед запуском). Валидируем, при
/// необходимости обновляем токен (мутирует `account` — вызывающий сохраняет).
pub async fn ensure_session(
    client: &reqwest::Client,
    base: &str,
    account: &mut Account,
) -> Result<()> {
    if ygg_validate(client, base, account).await? {
        return Ok(());
    }
    ygg_refresh(client, base, account).await
}

/// Загрузить скин (PNG-байты уже проверены `skin::validate_skin`). `slim` —
/// тонкая модель (Alex), иначе классическая (Steve).
pub async fn upload_skin(
    client: &reqwest::Client,
    base: &str,
    account: &Account,
    png: &[u8],
    slim: bool,
) -> Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    let body = json!({
        "skinBase64": b64,
        "skinModel": if slim { "slim" } else { "classic" },
    });
    let resp = client
        .patch(format!("{}/players/{}", rest_url(base), account.player_uuid))
        .bearer_auth(&account.api_token)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    Ok(())
}

/// Сменить ТОЛЬКО тип модели (slim/classic) у уже загруженного скина, без
/// пере-загрузки PNG. drasl принимает частичный PATCH профиля.
pub async fn set_skin_model(
    client: &reqwest::Client,
    base: &str,
    account: &Account,
    slim: bool,
) -> Result<()> {
    let body = json!({
        "skinModel": if slim { "slim" } else { "classic" },
    });
    let resp = client
        .patch(format!("{}/players/{}", rest_url(base), account.player_uuid))
        .bearer_auth(&account.api_token)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(api_error(resp).await);
    }
    Ok(())
}

/// Текущий URL скина игрока (для превью), через drasl REST.
pub async fn skin_url(
    client: &reqwest::Client,
    base: &str,
    account: &Account,
) -> Result<Option<String>> {
    let resp = client
        .get(format!("{}/players/{}", rest_url(base), account.player_uuid))
        .bearer_auth(&account.api_token)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: serde_json::Value = resp.json().await?;
    Ok(v["skinUrl"].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
}

impl Account {
    pub fn info(&self, skin_url: Option<String>) -> AccountInfo {
        AccountInfo {
            username: self.username.clone(),
            player_name: self.player_name.clone(),
            uuid: self.mc_uuid.clone(),
            skin_url,
        }
    }
}
