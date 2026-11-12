//! Static content copied from `design/directions/harman/index.html`. EN and TR strings are
//! hard-coded here; Fluent arrives in M1.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Tr,
}

const EN: &[(&str, &str)] = &[
    ("acc.ro", "read-only"),
    ("path.directCap", "Direct (P2P)"),
    ("path.direct", "direct (P2P)"),
    ("path.directShort", "Direct"),
    ("palette", "Search or run a command"),
    ("view.workspace", "Workspace"),
    ("view.splash", "Splash"),
    ("explorer", "Explorer"),
    ("filterConn", "Filter connections"),
    ("group.local", "Local"),
    ("tables", "Tables"),
    ("views", "Views"),
    ("functions", "Functions"),
    ("sequences", "Sequences"),
    ("connection", "connection"),
    ("kv.env", "Environment"),
    ("kv.access", "Access"),
    ("kv.path", "Path"),
    ("kv.agent", "Agent"),
    ("kv.engine", "Engine"),
    ("kv.granted", "Granted by"),
    ("kv.audit", "Audit"),
    ("audit.val", "signed at agent"),
    ("env.production", "production"),
    ("statements", "statements"),
    ("run", "Run"),
    ("explain", "Explain"),
    ("format", "Format"),
    ("autocommit", "Auto-commit"),
    ("limit", "Limit"),
    ("executed", "executed"),
    ("rowsWord", "rows"),
    ("notInScope", "not in scope"),
    ("kind.function", "function"),
    ("kind.kwfn", "keyword · function"),
    ("kind.keyword", "keyword"),
    ("hint.insert", "insert"),
    ("hint.replace", "replace"),
    ("hint.docs", "docs"),
    ("result1", "Result 1"),
    ("of", "of"),
    ("env.stagingParen", "(staging)"),
    ("countRows", "Count rows"),
    ("export", "Export"),
    ("roNote", "Editing disabled · production is read-only"),
    ("deleteRows", "Delete rows"),
    ("nextReady", "Next page ready"),
    ("prefetched", "prefetched"),
    ("selection", "Selection"),
    ("record", "Record"),
    ("rowWord", "row"),
    ("selRange", "Selected range"),
    ("cells", "cells"),
    ("avg", "avg"),
    ("avgPerOrder", "avg / order"),
    ("status.env", "production · read-only"),
    ("lncol", "Ln 14, Col 49"),
    ("env.prodCap", "production"),
    ("d.column", "column"),
    ("d.type", "Type"),
    ("d.nullable", "Nullable"),
    ("d.table", "Table"),
    ("d.refs", "References"),
    ("d.no", "no · NOT NULL"),
];

const TR: &[(&str, &str)] = &[
    ("acc.ro", "salt okunur"),
    ("path.directCap", "Doğrudan (P2P)"),
    ("path.direct", "doğrudan (P2P)"),
    ("path.directShort", "Doğrudan"),
    ("palette", "Ara veya komut çalıştır"),
    ("view.workspace", "Çalışma alanı"),
    ("view.splash", "Açılış"),
    ("explorer", "Gezgin"),
    ("filterConn", "Bağlantılarda ara"),
    ("group.local", "Yerel"),
    ("tables", "Tablolar"),
    ("views", "Görünümler"),
    ("functions", "Fonksiyonlar"),
    ("sequences", "Diziler"),
    ("connection", "bağlantı"),
    ("kv.env", "Ortam"),
    ("kv.access", "Erişim"),
    ("kv.path", "Yol"),
    ("kv.agent", "Ajan"),
    ("kv.engine", "Motor"),
    ("kv.granted", "Yetki veren"),
    ("kv.audit", "Denetim"),
    ("audit.val", "ajan tarafından imzalanır"),
    ("env.production", "üretim"),
    ("statements", "ifade"),
    ("run", "Çalıştır"),
    ("explain", "Planı göster"),
    ("format", "Biçimlendir"),
    ("autocommit", "Otomatik commit"),
    ("limit", "Sınır"),
    ("executed", "çalıştı"),
    ("rowsWord", "satır"),
    ("notInScope", "kapsam dışı"),
    ("kind.function", "fonksiyon"),
    ("kind.kwfn", "anahtar kelime · fonksiyon"),
    ("kind.keyword", "anahtar kelime"),
    ("hint.insert", "ekle"),
    ("hint.replace", "değiştir"),
    ("hint.docs", "belgeler"),
    ("result1", "Sonuç 1"),
    ("of", "/"),
    ("env.stagingParen", "(hazırlık)"),
    ("countRows", "Satırları say"),
    ("export", "Dışa aktar"),
    ("roNote", "Düzenleme kapalı · üretim salt okunur"),
    ("deleteRows", "Satırları sil"),
    ("nextReady", "Sonraki sayfa hazır"),
    ("prefetched", "önceden yüklendi"),
    ("selection", "Seçim"),
    ("record", "Kayıt"),
    ("rowWord", "satır"),
    ("selRange", "Seçili aralık"),
    ("cells", "hücre"),
    ("avg", "ortalama"),
    ("avgPerOrder", "ortalama / sipariş"),
    ("status.env", "üretim · salt okunur"),
    ("lncol", "Satır 14, Sütun 49"),
    ("env.prodCap", "prod"),
    ("d.column", "sütun"),
    ("d.type", "Tür"),
    ("d.nullable", "NULL olabilir"),
    ("d.table", "Tablo"),
    ("d.refs", "Başvuru"),
    ("d.no", "hayır · NOT NULL"),
];

/// Looks up a UI string; falls back to English, then panics (a missing key is a bug in the spike).
pub fn t(lang: Lang, key: &str) -> &'static str {
    let table = if lang == Lang::Tr { TR } else { EN };
    table
        .iter()
        .chain(EN.iter())
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| panic!("missing string {key}"))
}

/// Locale-aware upper case (`toLocaleUpperCase`): Turkish maps `i` to `İ`. Small-caps labels are
/// cased here, upstream of `hacivat_ui_core::fonts::small_caps`, which never changes case.
pub fn upper(lang: Lang, s: &str) -> String {
    s.chars()
        .flat_map(|ch| {
            let up: Vec<char> = if lang == Lang::Tr && ch == 'i' {
                vec!['\u{130}']
            } else {
                ch.to_uppercase().collect()
            };
            up
        })
        .collect()
}

/// `Intl.NumberFormat` equivalent for the two languages: `,`/`.` swap in Turkish.
pub fn nf(lang: Lang, v: f64, dp: usize) -> String {
    let s = format!("{v:.dp$}");
    let (int, frac) = s.split_once('.').map_or((s.as_str(), None), |(a, b)| (a, Some(b)));
    let (gsep, dsep) = if lang == Lang::Tr { ('.', ',') } else { (',', '.') };
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(gsep);
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push(dsep);
        out.push_str(f);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Prod,
    Stg,
    Loc,
}

pub struct TreeRow {
    /// A group header: (label key or raw text, raw?, count, agent radio icon)
    pub group: Option<(&'static str, bool, &'static str, bool)>,
    pub depth: usize,
    pub chev: Option<&'static str>,
    pub icon: &'static str,
    pub label: &'static str,
    pub i18n: bool,
    pub strong: bool,
    pub dim: bool,
    pub env: Option<Env>,
    pub ro: bool,
    pub p2p: bool,
    pub sel: bool,
    pub cnt: &'static str,
    pub meta: &'static str,
}

const ROW: TreeRow = TreeRow {
    group: None,
    depth: 0,
    chev: None,
    icon: "",
    label: "",
    i18n: false,
    strong: false,
    dim: false,
    env: None,
    ro: false,
    p2p: false,
    sel: false,
    cnt: "",
    meta: "",
};

pub fn tree() -> Vec<TreeRow> {
    let g = |label, raw, n, agent| TreeRow { group: Some((label, raw, n, agent)), ..ROW };
    let r = |depth, chev, icon, label| TreeRow { depth, chev, icon, label, ..ROW };
    vec![
        g("group.local", false, "1", false),
        TreeRow { env: Some(Env::Loc), meta: "sqlite", ..r(0, Some("down"), "database", "local-sqlite") },
        TreeRow { dim: true, ..r(1, None, "cylinder", "app.db") },
        g("Perde · Acme", true, "4", true),
        TreeRow {
            strong: true,
            env: Some(Env::Prod),
            ro: true,
            p2p: true,
            meta: "pg 16",
            ..r(0, Some("down"), "database", "orders-prod")
        },
        r(1, Some("down"), "cylinder", "orders"),
        r(2, Some("down"), "layers", "public"),
        TreeRow { i18n: true, cnt: "5", ..r(3, Some("down"), "table-properties", "tables") },
        r(4, None, "table-2", "customers"),
        TreeRow { sel: true, ..r(4, None, "table-2", "orders") },
        r(4, None, "table-2", "order_items"),
        r(4, None, "table-2", "products"),
        r(4, None, "table-2", "audit_log"),
        TreeRow { i18n: true, cnt: "3", ..r(3, Some("right"), "eye", "views") },
        TreeRow { i18n: true, cnt: "12", ..r(3, Some("right"), "square-function", "functions") },
        TreeRow { i18n: true, cnt: "5", ..r(3, Some("right"), "list-ordered", "sequences") },
        r(2, Some("down"), "layers", "billing"),
        TreeRow { i18n: true, cnt: "3", ..r(3, Some("down"), "table-properties", "tables") },
        r(4, None, "table-2", "invoices"),
        r(4, None, "table-2", "payments"),
        r(4, None, "table-2", "refunds"),
        TreeRow { env: Some(Env::Stg), meta: "pg 16", ..r(0, Some("right"), "database", "orders-staging") },
        TreeRow {
            env: Some(Env::Prod),
            ro: true,
            meta: "redis 7.4",
            ..r(0, Some("right"), "database", "cache-prod")
        },
        TreeRow { env: Some(Env::Stg), meta: "mongo 7", ..r(0, Some("right"), "database", "events") },
    ]
}

/// Result 1 rows: (full_name, city, orders, revenue); city None is NULL, Some("") is the empty string.
pub const R1: &[(&str, Option<&str>, u32, f64)] = &[
    ("Ada Yılmaz", Some("İstanbul"), 14, 48920.50),
    ("Bora Işık", Some("Ankara"), 12, 41388.00),
    ("Cem Öztürk", Some("İzmir"), 11, 39104.75),
    ("Deniz Çelik", Some("Berlin"), 9, 36870.20),
    ("Elif Şahin", Some("İstanbul"), 10, 34215.90),
    ("Furkan Güneş", Some("Iğdır"), 8, 31602.40),
    ("Gizem Aydın", Some("London"), 9, 29948.15),
    ("Hakan Doğan", None, 7, 27310.00),
    ("İpek Karaca", Some("Bursa"), 8, 25774.60),
    ("Jale Koç", Some("Antalya"), 6, 24190.35),
    ("Kerem Arslan", Some("Ankara"), 7, 22846.80),
    ("Lale Erdem", Some("Rotterdam"), 6, 21402.10),
    ("Mert Kılıç", Some("İstanbul"), 5, 19995.00),
    ("Nur Polat", Some("Eskişehir"), 6, 18730.45),
    ("Oğuz Tekin", Some("Berlin"), 5, 17264.90),
    ("Pınar Uçar", Some("İzmir"), 4, 16118.25),
    ("Rıza Kurt", Some(""), 5, 15402.70),
    ("Selin Avcı", Some("London"), 4, 14889.00),
    ("Tuna Bulut", Some("Kayseri"), 3, 13550.40),
    ("Umut Ekinci", Some("Gaziantep"), 4, 12906.15),
    ("Vural Şen", Some("Ankara"), 3, 12011.80),
];

pub const SEG1: &[&str] = &[
    "enterprise", "enterprise", "enterprise", "mid-market", "enterprise", "mid-market",
    "mid-market", "mid-market", "mid-market", "mid-market", "smb", "mid-market", "smb", "smb",
    "smb", "smb", "smb", "smb", "smb", "smb", "smb",
];

/// Column widths of the Result 1 grid (row-number column first); the last column fills.
pub const C1: [f32; 6] = [46.0, 232.0, 180.0, 132.0, 110.0, 160.0];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_format_matches_intl() {
        assert_eq!(nf(Lang::En, 48920.5, 2), "48,920.50");
        assert_eq!(nf(Lang::Tr, 48920.5, 2), "48.920,50");
        assert_eq!(nf(Lang::En, 2481.0, 0), "2,481");
        assert_eq!(nf(Lang::En, 100.0, 0), "100");
    }

    #[test]
    fn upper_is_locale_aware() {
        assert_eq!(upper(Lang::En, "Explorer"), "EXPLORER");
        assert_eq!(upper(Lang::En, "null"), "NULL");
        assert_eq!(upper(Lang::Tr, "Gezgin"), "GEZGİN");
        assert_eq!(upper(Lang::Tr, "Seçili aralık"), "SEÇİLİ ARALIK");
        assert_eq!(upper(Lang::Tr, "Kayıt"), "KAYIT");
    }

    #[test]
    fn every_en_key_has_a_tr_translation() {
        for (k, _) in EN {
            assert!(TR.iter().any(|(tk, _)| tk == k), "{k}");
        }
    }
}
