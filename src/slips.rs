use crate::escpos::{Escpos, Side, wrap};
use serde::Deserialize;

/// An order as the web app has it (restora-web `types/order`); fields a slip does not use are ignored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub daily_token: serde_json::Value,
    #[serde(rename = "type")]
    pub kind: String,
    pub items: Vec<Item>,
    #[serde(default)]
    pub subtotal: f64,
    #[serde(default)]
    pub discount: f64,
    #[serde(default)]
    pub tax_rate: f64,
    #[serde(default)]
    pub tax: f64,
    #[serde(default)]
    pub delivery_fee: f64,
    #[serde(default)]
    pub total: f64,
    pub payment_method: Option<String>,
    #[serde(default)]
    pub payment_status: String,
    pub table_name: Option<String>,
    pub area: Option<String>,
    pub address: Option<String>,
    pub rider: Option<String>,
    pub taken_by: Option<String>,
    pub customer: Option<Customer>,
    #[serde(default)]
    pub created_at: String,
    /// Set on a cancelled order: who cancelled it and why.
    pub cancellation: Option<Cancellation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cancellation {
    pub reason: Option<String>,
    pub note: Option<String>,
    pub by_name: Option<String>,
}

/// The cancel reasons as staff read them (restora-backend `constants/cancel_reasons`).
fn reason_text(cancellation: &Cancellation) -> Option<String> {
    let reason = cancellation.reason.as_deref().map(|code| match code {
        "customer_left" => "Customer left",
        "wrong_order" => "Wrong order",
        "out_of_stock" => "Out of stock",
        "duplicate" => "Duplicate order",
        _ => "Other",
    });
    let note = cancellation.note.as_deref().filter(|n| !n.is_empty());
    match (reason, note) {
        (Some(r), Some(n)) => Some(format!("{r} - {n}")),
        (Some(r), None) => Some(r.to_string()),
        (None, Some(n)) => Some(n.to_string()),
        (None, None) => None,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    pub qty: u32,
    #[serde(default)]
    pub price: f64,
    pub size_name: Option<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub addons: Vec<Addon>,
    #[serde(default)]
    pub includes: Vec<Included>,
}

#[derive(Deserialize)]
pub struct Addon {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Included {
    pub name: String,
    pub size_name: Option<String>,
    pub qty: u32,
}

#[derive(Deserialize)]
pub struct Customer {
    pub name: String,
    pub phone: Option<String>,
}

/// What the receipt header and footer carry; the web app sends it with the order (from `/me` workplace).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    #[serde(default)]
    pub restaurant: String,
    /// Branch name and address on one line.
    pub branch: Option<String>,
    pub phone: Option<String>,
    pub ntn: Option<String>,
    pub strn: Option<String>,
    pub footer: Option<String>,
}

/// 12345.6 -> "12,346"
fn number(amount: f64) -> String {
    let whole = amount.round() as i64;
    let digits = whole.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if whole < 0 { format!("-{out}") } else { out }
}

fn rs(amount: f64) -> String {
    format!("Rs {}", number(amount))
}

fn token(order: &Order) -> String {
    match &order.daily_token {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn where_line(order: &Order) -> String {
    let kind = match order.kind.as_str() {
        "dine_in" => "Dine in",
        "takeaway" => "Takeaway",
        "delivery" => "Delivery",
        other => other,
    };
    match &order.table_name {
        Some(table) => format!("{kind} - Table {table}"),
        None => kind.to_string(),
    }
}

/// Size and add-ons of a line, as `line_details.ts` writes them: "(2x Naan, Coke) Full - Raita".
fn details(item: &Item) -> String {
    let includes: Vec<String> = item
        .includes
        .iter()
        .map(|part| {
            let qty = if part.qty > 1 {
                format!("{}x ", part.qty)
            } else {
                String::new()
            };
            let size = part
                .size_name
                .as_deref()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default();
            format!("{qty}{}{size}", part.name)
        })
        .collect();
    let addons: Vec<&str> = item.addons.iter().map(|a| a.name.as_str()).collect();
    [
        (!includes.is_empty()).then(|| includes.join(", ")),
        item.size_name.clone().filter(|s| !s.is_empty()),
        (!addons.is_empty()).then(|| addons.join(", ")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" - ")
}

fn details_block(p: &mut Escpos, order: &Order, receipt: bool) {
    p.line(&format!(
        "Date  {}",
        order.created_at.get(..16).unwrap_or("").replace('T', " ")
    ));
    if let Some(customer) = &order.customer {
        p.line(&format!("Customer  {}", customer.name));
        if let (true, Some(phone)) = (receipt, &customer.phone) {
            p.line(&format!("Phone  {phone}"));
        }
    }
    if order.kind == "delivery" {
        let address: Vec<&str> = [order.area.as_deref(), order.address.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if !address.is_empty() {
            p.line(&format!("Address  {}", address.join(", ")));
        }
    }
    if let Some(rider) = &order.rider {
        p.line(&format!("Rider  {rider}"));
    }
}

/// The bytes of a slip, or nothing for a kind that is not kitchen or receipt.
pub fn render(kind: &str, order: &Order, header: &Header, width: usize) -> Option<Vec<u8>> {
    match kind {
        "kitchen" => Some(kitchen(order, width)),
        "receipt" => Some(receipt(order, header, width)),
        _ => None,
    }
}

/// The kitchen slip: what to cook, big, no prices.
pub fn kitchen(order: &Order, width: usize) -> Vec<u8> {
    let mut p = Escpos::new(width);
    p.align(Side::Center);
    p.bold(true);
    p.size(2);
    p.line("KITCHEN");
    p.size(1);
    p.bold(false);
    p.rule('-');
    p.align(Side::Left);
    p.bold(true);
    p.size(2);
    p.line(&format!("#{}", token(order)));
    p.size(1);
    p.line(&where_line(order));
    p.bold(false);
    details_block(&mut p, order, false);
    p.rule('-');
    if let Some(cancellation) = &order.cancellation {
        // A big cross, then who cancelled it and why: nothing on this slip is to be made.
        p.align(Side::Center);
        p.bold(true);
        p.size(2);
        for row in ["X     X", " X   X ", "  X X  ", "   X   ", "  X X  ", " X   X ", "X     X"] {
            p.line(row);
        }
        p.invert(true);
        p.line(" CANCELLED ");
        p.invert(false);
        p.size(1);
        p.align(Side::Left);
        if let Some(by) = &cancellation.by_name {
            p.line(&format!("By  {by}"));
        }
        if let Some(reason) = reason_text(cancellation) {
            for text in wrap(&format!("Reason: {reason}"), width) {
                p.line(&text);
            }
        }
        p.bold(false);
        p.rule('-');
    }
    for item in &order.items {
        // A line to stop making (CANCEL, from an edit or a cancel) prints white on black.
        let cut = item.name.starts_with("CANCEL ");
        p.invert(cut);
        p.bold(true);
        p.size(2);
        for (i, text) in wrap(&format!("{}x {}", item.qty, item.name), width / 2)
            .iter()
            .enumerate()
        {
            p.line(&if i == 0 {
                text.clone()
            } else {
                format!("   {text}")
            });
        }
        p.size(1);
        p.bold(false);
        let extra = details(item);
        for text in wrap(&extra, width.saturating_sub(3)).iter().chain(
            &item
                .note
                .as_deref()
                .map(|n| wrap(&format!("\"{n}\""), width - 3))
                .unwrap_or_default(),
        ) {
            p.line(&format!("   {text}"));
        }
        p.invert(false);
    }
    p.rule('-');
    p.cut();
    p.bytes
}

/// The customer's receipt. Optional lines are left off while empty.
pub fn receipt(order: &Order, header: &Header, width: usize) -> Vec<u8> {
    let mut p = Escpos::new(width);
    p.align(Side::Center);
    p.bold(true);
    p.size(2);
    for text in wrap(&header.restaurant, width / 2) {
        p.line(&text);
    }
    p.size(1);
    p.bold(false);
    for text in [&header.branch, &header.phone]
        .into_iter()
        .flatten()
        .flat_map(|t| wrap(t, width))
    {
        p.line(&text);
    }
    let tax_numbers: Vec<String> = [
        header.ntn.as_ref().map(|n| format!("NTN {n}")),
        header.strn.as_ref().map(|n| format!("STRN {n}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !tax_numbers.is_empty() {
        p.line(&tax_numbers.join(" - "));
    }
    p.rule('-');
    p.align(Side::Left);
    p.bold(true);
    p.pair(&format!("#{}", token(order)), &where_line(order));
    p.bold(false);
    details_block(&mut p, order, true);
    p.rule('-');

    for item in &order.items {
        p.pair(
            &format!("{}x {}", item.qty, item.name),
            &number(item.price * item.qty as f64),
        );
        let extra = details(item);
        for text in wrap(&extra, width.saturating_sub(3)).iter().chain(
            &item
                .note
                .as_deref()
                .map(|n| wrap(&format!("\"{n}\""), width - 3))
                .unwrap_or_default(),
        ) {
            p.line(&format!("   {text}"));
        }
    }
    p.rule('-');
    p.pair("Subtotal", &rs(order.subtotal));
    if order.discount > 0.0 {
        p.pair("Discount", &format!("- {}", rs(order.discount)));
    }
    if order.payment_method.is_some() {
        p.pair(&format!("Tax ({}%)", order.tax_rate), &rs(order.tax));
    } else {
        p.pair("Tax", "Added when paid");
    }
    if order.delivery_fee > 0.0 {
        p.pair("Delivery", &rs(order.delivery_fee));
    }
    p.bold(true);
    p.size(2);
    p.pair("Total", &rs(order.total));
    p.size(1);
    p.bold(false);
    if order.payment_status == "paid" {
        p.align(Side::Center);
        p.bold(true);
        p.line(&format!(
            "*** PAID{} ***",
            order
                .payment_method
                .as_deref()
                .map(|m| format!(" ({})", m.to_uppercase()))
                .unwrap_or_default()
        ));
        p.bold(false);
    }
    p.align(Side::Center);
    p.rule('-');
    for text in wrap(
        header
            .footer
            .as_deref()
            .unwrap_or("Thank you! Please come again."),
        width,
    ) {
        p.line(&text);
    }
    if let Some(name) = &order.taken_by {
        p.line(&format!("Served by {name}"));
    }
    p.cut();
    p.bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Order {
        serde_json::from_value(serde_json::json!({
            "dailyToken": 12, "type": "dine_in", "tableName": "T4", "createdAt": "2026-10-27T17:19:09Z",
            "subtotal": 2500, "discount": 0, "taxRate": 16, "tax": 400, "total": 2900, "paymentStatus": "paid", "paymentMethod": "cash",
            "items": [
                { "name": "Chicken Karahi", "qty": 1, "price": 1800, "sizeName": "Half", "note": "less spicy" },
                { "name": "Naan", "qty": 2, "price": 350, "addons": [{ "name": "Butter" }] }
            ]
        }))
        .unwrap()
    }

    /// The printable text of a slip: ESC/POS commands removed.
    fn text(bytes: &[u8]) -> String {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                0x1b if bytes.get(i + 1) == Some(&b'@') => i += 2,
                0x1b => i += 3,
                0x1d if bytes.get(i + 1) == Some(&b'V') => i += 4,
                0x1d => i += 3,
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn kitchen_slip_has_dishes_and_no_prices() {
        let slip = text(&kitchen(&order(), 48));
        assert!(slip.contains("#12") && slip.contains("Dine in - Table T4"));
        assert!(
            slip.contains("1x Chicken Karahi")
                && slip.contains("Half")
                && slip.contains("\"less spicy\"")
        );
        assert!(slip.contains("2x Naan") && slip.contains("Butter"));
        assert!(!slip.contains("Rs") && !slip.contains("1,800"));
    }

    #[test]
    fn cancelled_slip_has_a_cross_who_and_why_and_cut_lines() {
        let order: Order = serde_json::from_value(serde_json::json!({
            "dailyToken": "12 CANCELLED", "type": "takeaway", "createdAt": "2026-10-31T08:00:00Z",
            "items": [{ "name": "CANCEL Burger", "qty": 4 }],
            "cancellation": { "reason": "customer_left", "note": "changed mind", "byName": "Ali" }
        }))
        .unwrap();
        let bytes = kitchen(&order, 48);
        let slip = text(&bytes);
        assert!(slip.contains("#12 CANCELLED") && slip.contains("X     X") && slip.contains(" CANCELLED "));
        assert!(slip.contains("By  Ali") && slip.contains("Reason: Customer left - changed mind"));
        assert!(slip.contains("4x CANCEL Burger"));
        // The cut line is white on black.
        assert!(bytes.windows(3).any(|w| w == [0x1d, b'B', 1]));
    }

    #[test]
    fn a_slip_without_cancel_lines_is_never_white_on_black() {
        let bytes = kitchen(&order(), 48);
        assert!(!bytes.windows(3).any(|w| w == [0x1d, b'B', 1]));
    }

    #[test]
    fn receipt_has_totals_and_fits_the_roll() {
        let header = Header {
            restaurant: "Karahi House".into(),
            ntn: Some("123".into()),
            ..Header::default()
        };
        let bytes = receipt(&order(), &header, 32);
        let slip = text(&bytes);
        assert!(
            slip.contains("Rs 2,900")
                && slip.contains("Tax (16%)")
                && slip.contains("*** PAID (CASH) ***")
        );
        assert!(slip.contains("NTN 123") && slip.contains("Thank you! Please come again."));
        // Double-size lines (header, total) use half the characters; every other line fits 32.
        assert!(slip.lines().all(|l| l.len() <= 32), "{slip}");
        assert!(bytes.ends_with(&[0x1d, b'V', 66, 0]));
    }
}
