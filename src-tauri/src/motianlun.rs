use crate::{
    http::{self, first, string, Account},
    tasks::{Outcome, TaskContext},
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Map, Value};

const API: &str = "https://unify.moretickets.com";
const SITE: &str = "https://www.moretickets.com";
const MOTIANLUN: &str = "https://www.motianlun.cn";
const ORDERS: &str = "https://www.moretickets.com/my-order";
const VERSION: &str = "6.86.0";

const ELECTRONIC: &[&str] = &[
    "E_TICKET",
    "TICKET_TRANSFER",
    "CUSTOMER_REDEMPTION",
    "REALNAME_VOUCHER",
    "ON_SITE",
];

pub fn orders_url() -> &'static str {
    ORDERS
}

pub fn project_url(project_id: &str) -> String {
    format!("{SITE}/show-detail?showId={project_id}")
}

fn show_id(id: &str) -> Result<(), String> {
    if id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("请输入 24 位摩天轮或 MoreTickets 项目编号".into())
    }
}

pub fn access_token(cookie: &str) -> String {
    let token = http::cookie_value(cookie, "access_token");
    if token.is_empty() {
        http::cookie_value(cookie, "access-token")
    } else {
        token
    }
}

fn client(account: &Account) -> Result<Client, String> {
    if access_token(&account.cookie).is_empty() {
        return Err(
            "Cookie 缺少 access_token。请在 moretickets.com 或摩天轮页面的 Network 请求头中复制 access_token（网页）或 access-token（App 内网页），按 access_token=...; tsessionid=... 保存"
                .into(),
        );
    }
    http::client(account, SITE)
}

fn authed(
    builder: reqwest::RequestBuilder,
    account: &Account,
) -> Result<reqwest::RequestBuilder, String> {
    let token = access_token(&account.cookie);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("src", "PC".parse().map_err(|_| "请求头无效")?);
    headers.insert("channel", "PC".parse().map_err(|_| "请求头无效")?);
    headers.insert("ver", VERSION.parse().map_err(|_| "请求头无效")?);
    headers.insert("lan", "zh-CN".parse().map_err(|_| "请求头无效")?);
    headers.insert(
        "access_token",
        token.parse().map_err(|_| "access_token 包含无效字符")?,
    );
    headers.insert(
        "access-token",
        token.parse().map_err(|_| "access_token 包含无效字符")?,
    );
    let session = http::cookie_value(&account.cookie, "tsessionid");
    if !session.is_empty() {
        headers.insert(
            "tsessionid",
            session.parse().map_err(|_| "tsessionid 包含无效字符")?,
        );
    }
    Ok(builder.headers(headers))
}

async fn get(account: &Account, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    let api = client(account)?;
    data(
        http::json(authed(
            api.get(format!("{API}{path}")).query(query),
            account,
        )?)
        .await?,
    )
}

async fn post(account: &Account, path: &str, body: &Value) -> Result<Value, String> {
    let api = client(account)?;
    data(
        http::json(authed(
            api.post(format!("{API}{path}")).json(body),
            account,
        )?)
        .await?,
    )
}

fn data(response: Value) -> Result<Value, String> {
    let code = response["statusCode"].as_i64().unwrap_or(-1);
    if code == 200 {
        return Ok(response["data"].clone());
    }
    let message = string(first(&response, &["message", "comments"]));
    Err(if message.is_empty() {
        format!("摩天轮接口返回 {code}")
    } else {
        format!("{message}（{code}）")
    })
}

fn missing_show(message: &str) -> bool {
    message.contains("不存在") || message.contains("210001")
}

fn terminal(message: &str) -> bool {
    [
        "Token",
        "token",
        "登录",
        "12315",
        "12306",
        "限购",
        "实名",
        "观演人",
        "配送",
        "地址",
        "支付方式",
        "票价",
        "已下架",
        "已结束",
        "未开售",
    ]
    .iter()
    .any(|word| message.contains(word))
}

fn show_id_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "showId" || key == "showOID")
                .map(|(_, value)| value.into_owned())
        })
        .filter(|id| show_id(id).is_ok())
        .unwrap_or_default()
}

async fn unify_show(account: &Account, project_id: &str) -> Result<Value, String> {
    get(
        account,
        "/user/subject/show/v1/detail",
        &[("showId", project_id)],
    )
    .await
}

async fn sessions(account: &Account, project_id: &str) -> Result<Vec<Value>, String> {
    let data = post(
        account,
        "/user/foundation/pub/session/list/v1",
        &json!({ "showId": project_id, "completedSession": false }),
    )
    .await?;
    Ok(data["sessionList"].as_array().cloned().unwrap_or_default())
}

async fn motianlun_show(account: &Account, project_id: &str) -> Result<Value, String> {
    let api = http::client(account, MOTIANLUN)?;
    let response = http::json(
        api.get(format!("{MOTIANLUN}/showapi/pub/show/{project_id}"))
            .query(&[("src", "web"), ("ver", VERSION)]),
    )
    .await?;
    let code = response["statusCode"].as_i64().unwrap_or(-1);
    if code != 200 {
        return data(response);
    }
    let nested = &response["result"]["data"];
    let show = if response["data"].is_object() {
        response["data"].clone()
    } else if nested.is_object() {
        nested.clone()
    } else {
        return Err("摩天轮未返回演出详情".into());
    };
    Ok(show)
}

fn as_unify_show(show: &Value, project_id: &str) -> Value {
    let status = &show["showStatus"];
    let id = string(first(show, &["showOID", "id"]));
    json!({
        "id": if id.is_empty() { project_id } else { id.as_str() },
        "showName": string(first(show, &["showName"])),
        "venueName": string(&show["venueName"]),
        "cityName": string(&show["cityName"]),
        "posterUrl": string(first(show, &["posterURL", "posterUrl"])),
        "showStatus": if status.is_object() { string(&status["name"]) } else { string(status) },
    })
}

#[tauri::command]
pub async fn mtl_project(account: Account, project_id: String) -> Result<Value, String> {
    show_id(&project_id)?;
    let _ = client(&account)?;
    match unify_show(&account, &project_id).await {
        Ok(show) => {
            let sessions = sessions(&account, &project_id).await?;
            let note = if sessions.is_empty() {
                "已加载演出，但公开场次接口没有返回可售场次。"
            } else {
                ""
            };
            Ok(json!({ "source": "moretickets", "show": show, "sessions": sessions, "note": note }))
        }
        Err(error) if missing_show(&error) => {
            let show = motianlun_show(&account, &project_id)
                .await
                .map_err(|fallback| format!("{error}；摩天轮站内详情也未返回：{fallback}"))?;
            let jumped = show_id_from_url(&string(&show["mtsShowVersionInfo"]["mtsOrderUrl"]));
            if !jumped.is_empty() && jumped != project_id {
                if let Ok(remote) = unify_show(&account, &jumped).await {
                    let sessions = sessions(&account, &jumped).await.unwrap_or_default();
                    return Ok(json!({
                        "source": "moretickets",
                        "show": remote,
                        "sessions": sessions,
                        "note": "该摩天轮演出由 MoreTickets 履约，已改用跨境项目编号加载场次。",
                    }));
                }
            }
            Ok(json!({
                "source": "motianlun",
                "show": as_unify_show(&show, &project_id),
                "sessions": [],
                "note": "已识别摩天轮站内演出，但公开页面没有核实到场次、库存和下单接口。MoreTickets / 跨境演出请改用 tourDetail 链接中的 showId；国内场次接口仍待实账号核对。",
            }))
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn mtl_tickets(
    account: Account,
    project_id: String,
    screen_id: String,
) -> Result<Value, String> {
    show_id(&project_id)?;
    show_id(&screen_id)?;
    let _ = client(&account)?;
    post(
        &account,
        "/user/inventory/list/v1",
        &json!({
            "showId": project_id,
            "sessionId": screen_id,
            "ticketNumber": 1,
            "seatedTogether": false,
        }),
    )
    .await
}

#[tauri::command]
pub async fn mtl_buyers(account: Account) -> Result<Value, String> {
    let _ = client(&account)?;
    let data = get(&account, "/user/v1/audience/list", &[]).await?;
    let list = data
        .as_array()
        .or_else(|| data.get("list").and_then(Value::as_array))
        .or_else(|| data.get("audiences").and_then(Value::as_array));
    list.cloned()
        .map(Value::Array)
        .ok_or_else(|| "未返回观演人列表，请先在 MoreTickets 或摩天轮添加实名信息".into())
}

#[derive(Debug, PartialEq)]
pub enum Availability {
    Available,
    Unavailable,
    Unknown,
}

pub fn inventory_availability(raw: &Value, sku_id: &str) -> Result<Availability, String> {
    let items = raw.as_array().ok_or("未返回有效库存列表")?;
    let Some(item) = items
        .iter()
        .find(|item| string(&item["inventoryId"]) == sku_id)
    else {
        return Ok(Availability::Unknown);
    };
    match item["stock"]
        .as_i64()
        .or_else(|| item["stock"].as_str().and_then(|s| s.parse().ok()))
    {
        Some(stock) if stock > 0 => Ok(Availability::Available),
        Some(_) => Ok(Availability::Unavailable),
        None => Ok(Availability::Unknown),
    }
}

pub async fn ticket_availability(
    account: &Account,
    project_id: &str,
    screen_id: &str,
    sku_id: &str,
) -> Result<Availability, String> {
    let raw = mtl_tickets(account.clone(), project_id.into(), screen_id.into()).await?;
    inventory_availability(&raw, sku_id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    #[serde(default)]
    row_name: String,
    #[serde(default)]
    row_num: Value,
    #[serde(default)]
    first_seat: Value,
    #[serde(default)]
    last_seat: Value,
    #[serde(default)]
    first_num: Value,
    #[serde(default)]
    last_num: Value,
    #[serde(default)]
    mts_zone_id: String,
    #[serde(default)]
    mts_category_id: String,
    #[serde(default)]
    mts_section_id: String,
    #[serde(default)]
    mts_zone_rs_id: String,
    #[serde(default)]
    origin_price: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    account: Account,
    project_id: String,
    screen_id: String,
    sku_id: String,
    count: u32,
    buyers: Vec<String>,
    #[serde(default)]
    listing: Listing,
}

impl Default for Listing {
    fn default() -> Self {
        Self {
            row_name: String::new(),
            row_num: Value::Null,
            first_seat: Value::Null,
            last_seat: Value::Null,
            first_num: Value::Null,
            last_num: Value::Null,
            mts_zone_id: String::new(),
            mts_category_id: String::new(),
            mts_section_id: String::new(),
            mts_zone_rs_id: String::new(),
            origin_price: String::new(),
        }
    }
}

pub fn validate(value: &Value) -> Result<(), String> {
    let config: Config =
        serde_json::from_value(value.clone()).map_err(|_| "摩天轮任务配置不完整")?;
    show_id(&config.project_id)?;
    show_id(&config.screen_id)?;
    show_id(&config.sku_id)?;
    if config.count == 0 || config.count > 20 || config.buyers.len() != config.count as usize {
        return Err("购买数量必须与观演人数一致，且最多 20 张".into());
    }
    let unique: std::collections::HashSet<_> =
        config.buyers.iter().filter(|id| !id.is_empty()).collect();
    if unique.len() != config.buyers.len() {
        return Err("观演人不能重复或为空".into());
    }
    client(&config.account)?;
    Ok(())
}

fn choose_delivery(methods: &[String]) -> Result<String, String> {
    if let Some(method) = ELECTRONIC
        .iter()
        .find(|name| methods.iter().any(|item| item == *name))
    {
        return Ok((*method).to_string());
    }
    if methods
        .iter()
        .any(|item| item == "EXPRESS" || item == "SELF_PICKUP")
    {
        return Err(
            "该库存需要快递或自取地址，当前版本只提交电子票/现场票。请在官方页面完成这类配送。"
                .into(),
        );
    }
    Err("订单确认没有返回可用的电子票或现场取票方式".into())
}

fn money_side(detail: &Value, side: &str) -> Value {
    let mut map = Map::new();
    for key in [
        "payableAmount",
        "ticketAmount",
        "serviceFee",
        "vat",
        "discountAmount",
    ] {
        if let Some(value) = detail
            .get(key)
            .and_then(|item| item.get(side))
            .filter(|v| !v.is_null())
        {
            map.insert(key.into(), value.clone());
        }
    }
    let currency_key = if side == "trade" {
        "tradeCurrency"
    } else {
        "displayCurrency"
    };
    if let Some(currency) = detail
        .get(currency_key)
        .and_then(|item| item.get("currency"))
        .filter(|v| !v.is_null())
    {
        map.insert("transactionCurrency".into(), currency.clone());
    }
    Value::Object(map)
}

fn selected_inventory<'a>(items: &'a Value, sku_id: &str) -> Option<&'a Value> {
    items
        .as_array()?
        .iter()
        .find(|item| string(&item["inventoryId"]) == sku_id)
}

pub fn order_id(data: &Value) -> Option<String> {
    for value in [data, &data["order"], &data["orderInfo"]] {
        let id = string(first(value, &["orderId", "order_id"]));
        if !id.is_empty() && id != "0" {
            return Some(id);
        }
    }
    None
}

fn order_page(id: &str) -> String {
    if id.is_empty() {
        ORDERS.into()
    } else {
        format!("{SITE}/order-detail?orderId={id}")
    }
}

pub async fn run(context: &TaskContext) -> Result<Outcome, String> {
    let config: Config =
        serde_json::from_value(context.request.config.clone()).map_err(|_| "任务参数无效")?;
    let page = project_url(&config.project_id);
    let mut last_error = String::new();
    for attempt in 1..=context.request.max_attempts {
        context.report("running", "正在核对库存与价格", attempt, None);
        let inventory = match mtl_tickets(
            config.account.clone(),
            config.project_id.clone(),
            config.screen_id.clone(),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                last_error = error;
                if terminal(&last_error) {
                    return Ok(Outcome::action(last_error, page));
                }
                context.report(
                    "running",
                    format!("{last_error}；等待下一次尝试"),
                    attempt,
                    None,
                );
                if attempt < context.request.max_attempts {
                    context.pause(context.request.interval_ms).await;
                }
                continue;
            }
        };
        let Some(item) = selected_inventory(&inventory, &config.sku_id) else {
            last_error = "所选库存已变化，请重新加载票档".into();
            return Ok(Outcome::action(last_error, page));
        };
        let stock = item["stock"].as_i64().unwrap_or(-1);
        if stock >= 0 && stock < i64::from(config.count) {
            last_error = "库存不足".into();
            context.report(
                "running",
                format!("{last_error}；等待下一次尝试"),
                attempt,
                None,
            );
            if attempt < context.request.max_attempts {
                context.pause(context.request.interval_ms).await;
            }
            continue;
        }
        let fresh_price = string(first(item, &["originPrice", "discountPrice"]));
        if !config.listing.origin_price.is_empty()
            && !fresh_price.is_empty()
            && fresh_price != config.listing.origin_price
        {
            return Ok(Outcome::action(
                "票价已变化，请重新加载票档并确认价格",
                page,
            ));
        }
        context.report("running", "正在确认配送方式并提交订单", attempt, None);
        let confirmation = post(
            &config.account,
            "/user/order/v1/confirmation/info",
            &json!({
                "showId": config.project_id,
                "sessionId": config.screen_id,
                "inventoryId": config.sku_id,
                "qty": config.count,
                "consecutive": false,
            }),
        )
        .await;
        let confirmation = match confirmation {
            Ok(value) => value,
            Err(error) => {
                last_error = error;
                if terminal(&last_error) {
                    return Ok(Outcome::action(last_error, ORDERS.into()));
                }
                context.report(
                    "running",
                    format!("{last_error}；等待下一次尝试"),
                    attempt,
                    None,
                );
                if attempt < context.request.max_attempts {
                    context.pause(context.request.interval_ms).await;
                }
                continue;
            }
        };
        let methods = confirmation["deliverMethods"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(string)
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let delivery = match choose_delivery(&methods) {
            Ok(method) => method,
            Err(error) => return Ok(Outcome::action(error, page)),
        };
        let quote = post(
            &config.account,
            "/user/order/v3/pre_order_detail",
            &json!({
                "showId": config.project_id,
                "sessionId": config.screen_id,
                "inventoryId": config.sku_id,
                "seatPlanId": config.listing.mts_category_id.clone(),
                "qty": config.count,
            }),
        )
        .await
        .unwrap_or(Value::Null);
        let audiences = match get(&config.account, "/user/v1/audience/list", &[]).await {
            Ok(value) => value,
            Err(error) => {
                last_error = error;
                if terminal(&last_error) {
                    return Ok(Outcome::action(last_error, page));
                }
                context.report(
                    "running",
                    format!("{last_error}；等待下一次尝试"),
                    attempt,
                    None,
                );
                if attempt < context.request.max_attempts {
                    context.pause(context.request.interval_ms).await;
                }
                continue;
            }
        };
        let people = audiences.as_array().cloned().unwrap_or_default();
        let mut selected = Vec::new();
        for id in &config.buyers {
            let Some(person) = people.iter().find(|item| string(&item["id"]) == *id) else {
                return Ok(Outcome::action(
                    "所选观演人已变化，请重新加载实名信息",
                    page,
                ));
            };
            selected.push(person.clone());
        }
        let mut payload = json!({
            "showId": config.project_id,
            "sessionId": config.screen_id,
            "inventoryId": config.sku_id,
            "qty": config.count,
            "consecutive": false,
            "rowName": if config.listing.row_name.is_empty() { string(&item["rowName"]) } else { config.listing.row_name.clone() },
            "rowNum": if config.listing.row_num.is_null() { item["rowNum"].clone() } else { config.listing.row_num.clone() },
            "firstSeat": config.listing.first_seat.clone(),
            "lastSeat": config.listing.last_seat.clone(),
            "firstNum": config.listing.first_num.clone(),
            "lastNum": config.listing.last_num.clone(),
            "mtsZoneId": config.listing.mts_zone_id,
            "mtsCategoryId": config.listing.mts_category_id,
            "mtsSectionId": config.listing.mts_section_id,
            "mtsZoneRsId": config.listing.mts_zone_rs_id,
            "delivery": { "deliverMethod": delivery },
            "audience": { "audiences": selected },
        });
        let trade = money_side(&quote, "trade");
        let display = money_side(&quote, "display");
        if trade.as_object().is_some_and(|map| !map.is_empty()) {
            payload["tradeAmount"] = trade;
            payload["displayAmount"] = display;
        }
        if let Some(method) = quote["payMethods"]
            .as_array()
            .and_then(|items| items.first())
            .map(|item| string(&item["payMethod"]))
            .filter(|item| !item.is_empty())
        {
            payload["payMethod"] = json!(method);
        }
        let created = match post(&config.account, "/user/order/v3/create", &payload).await {
            Ok(value) => value,
            Err(error) => {
                if error.contains("非 JSON") || error.contains("HTTP") {
                    return Ok(Outcome::action(
                        "订单请求结果未确认，请先检查 MoreTickets 订单页，避免重复下单",
                        ORDERS.into(),
                    ));
                }
                last_error = error;
                if terminal(&last_error) {
                    return Ok(Outcome::action(last_error, ORDERS.into()));
                }
                context.report(
                    "running",
                    format!("{last_error}；等待下一次尝试"),
                    attempt,
                    None,
                );
                if attempt < context.request.max_attempts {
                    context.pause(context.request.interval_ms).await;
                }
                continue;
            }
        };
        // 金额、支付方式和成功订单号仍需真实 Cookie 核对；没有订单号时不当作成功。
        let Some(id) = order_id(&created) else {
            return Ok(Outcome::action(
                "下单接口已响应，但没有返回可确认的订单号。请到官方订单页核对，避免重复提交",
                ORDERS.into(),
            ));
        };
        return Ok(Outcome {
            status: "succeeded",
            message: "订单已创建，请前往 MoreTickets 完成支付".into(),
            order_url: Some(order_page(&id)),
        });
    }
    Err(format!("已达到尝试次数上限：{last_error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(cookie: &str) -> Value {
        json!({
            "account": { "cookie": cookie },
            "projectId": "69fb10c1b0bcc10001a27774",
            "screenId": "6a016a61bb59430001084250",
            "skuId": "6ae3a9835983470001717b08",
            "count": 1,
            "buyers": ["audience-1"],
            "listing": { "originPrice": "2000", "mtsCategoryId": "category" }
        })
    }

    #[test]
    fn account_must_carry_an_access_token() {
        assert!(validate(&config("SESSION=only"))
            .unwrap_err()
            .contains("access_token"));
        assert!(validate(&config("access-token=web-view-token")).is_ok());
        assert_eq!(access_token("access_token=web; tsessionid=abc"), "web");
    }

    #[test]
    fn inventory_stock_is_positive_evidence() {
        let raw = json!([
            {"inventoryId":"6ae3a9835983470001717b08","stock":2},
            {"inventoryId":"6ae44b6fc5ca3d0001dbfc07","stock":"0"},
            {"inventoryId":"69fb10c1b0bcc10001a27775"}
        ]);
        assert_eq!(
            inventory_availability(&raw, "6ae3a9835983470001717b08").unwrap(),
            Availability::Available
        );
        assert_eq!(
            inventory_availability(&raw, "6ae44b6fc5ca3d0001dbfc07").unwrap(),
            Availability::Unavailable
        );
        assert_eq!(
            inventory_availability(&raw, "69fb10c1b0bcc10001a27775").unwrap(),
            Availability::Unknown
        );
        assert_eq!(
            inventory_availability(&raw, "missing").unwrap(),
            Availability::Unknown
        );
    }

    #[test]
    fn delivery_prefers_electronic_tickets() {
        assert_eq!(
            choose_delivery(&["EXPRESS".into(), "E_TICKET".into()]).unwrap(),
            "E_TICKET"
        );
        assert!(choose_delivery(&["EXPRESS".into()])
            .unwrap_err()
            .contains("快递"));
        assert!(choose_delivery(&[]).is_err());
    }

    #[test]
    fn created_orders_need_an_id() {
        assert_eq!(
            order_id(&json!({"orderId":"8891"})).as_deref(),
            Some("8891")
        );
        assert!(order_id(&json!({"orderId":""})).is_none());
        assert!(order_id(&json!({})).is_none());
    }
}
