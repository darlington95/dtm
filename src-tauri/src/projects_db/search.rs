use entity::images;
use sea_orm::{Condition, ExprTrait, QueryFilter, Select};
use sea_query::{Expr, SimpleExpr};

/// 検索モード:
/// - "auto":      既定。クエリ（正規化後）が ASCII かラテン文字のみなら "word"、
///   それ以外の文字（CJK・キリル・ギリシャ等）を含むなら "substring"。
/// - "word":      porter トークナイザーの FTS で語マッチ（stemming / prefix）。
///   CJK（漢字・日本語）の任意位置の部分一致はできない。
/// - "substring": trigram トークナイザーの FTS で文字列の部分一致。
///   漢字・日本語を含む CJK の検索に有効。
pub fn add_search(
    query: Select<images::Entity>,
    search_text: &str,
    mode: &str,
) -> Select<images::Entity> {
    let search_text = search_text.trim();
    if search_text.is_empty() {
        return query;
    }

    match mode {
        "substring" => add_search_substring(query, search_text),
        "auto" => {
            // 判定はインデックス・クエリに実際に渡る形（process_prompt 正規化後）で行う。
            // 全角ラテン文字は NFKC で ASCII 化されるため、ここで正規化しておく。
            let normalized = process_prompt(search_text);
            if is_ascii_or_latin(&normalized) {
                add_search_word(query, search_text)
            } else {
                add_search_substring(query, search_text)
            }
        }
        _ => add_search_word(query, search_text),
    }
}

/// クエリが ASCII かラテン文字（Unicode Script=Latin）のみで構成されるかを判定する。
/// Auto モードのルーティングに使用する。数字・記号・空白は ASCII として許容する。
fn is_ascii_or_latin(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_ascii() || is_latin_char(c))
}

/// Unicode Script=Latin に属する文字かどうか（検索クエリで現実的に現れる範囲）。
fn is_latin_char(c: char) -> bool {
    matches!(
        c as u32,
        // Latin-1 Supplement（é, ñ, ü, µ 等）
        0x00AA
            | 0x00B5
            | 0x00BA
            | 0x00C0..=0x00D6
            | 0x00D8..=0x00F6
            | 0x00F8..=0x00FF
        // Latin Extended-A / B（ą, č, ğ, ş, ž, ł 等）
            | 0x0100..=0x024F
        // IPA Extensions（ə 等）
            | 0x0250
            | 0x02A0..=0x02AF
        // Latin Extended Additional（ベトナム語 ộ, ẻ 等）
            | 0x1E00..=0x1EFF
        // Latin Extended-C / D / E / F
            | 0x2C60..=0x2C7F
            | 0xA720..=0xA7FF
            | 0xAB30..=0xAB6F
            | 0xABE3..=0xABEB
        // Latin ligatures（ﬁ, ﬂ 等）
            | 0xFB00..=0xFB06
        // 全角ラテン文字
            | 0xFF21..=0xFF3A
            | 0xFF41..=0xFF5A
    )
}

/// 語マッチモード（既存挙動）。
/// 引用語 `"..."` は LIKE の部分一致、残り語は porter FTS（OR）でマッチ。
fn add_search_word(
    mut query: Select<images::Entity>,
    search_text: &str,
) -> Select<images::Entity> {
    // 1. Extract quoted phrases
    let mut phrases = Vec::new();
    let mut remainder = String::new();

    let mut in_quotes = false;
    let mut current = String::new();

    for c in search_text.chars() {
        match c {
            '"' => {
                if in_quotes {
                    phrases.push(current.clone());
                    current.clear();
                }
                in_quotes = !in_quotes;
            }
            _ => {
                if in_quotes {
                    current.push(c);
                } else {
                    remainder.push(c);
                }
            }
        }
    }

    // 2. Apply phrase filters (AND)
    for phrase in &phrases {
        let like = format!("%{}%", phrase);
        query = query.filter(Expr::col(images::Column::Prompt).like(like));
    }

    // 3. Remaining terms → FTS OR query
    remainder = process_prompt(&remainder);
    let terms: Vec<&str> = remainder.split_whitespace().collect();

    if !terms.is_empty() {
        let fts_query = terms.join(" OR ");

        query = query.filter(Expr::cust_with_expr(
            "images.id IN (SELECT rowid FROM images_fts WHERE images_fts MATCH 'prompt_search:' || ?)",
            SimpleExpr::value(fts_query),
        ));
    }

    query
}

/// 文字列の部分一致モード。
/// 引用語・非引用語いずれも trigram FTS（`images_fts_trigram`、`prompt_search` 列限定）で部分一致。
/// 引用語は AND 結合、非引用語は OR 結合。
/// 3 文字未満のクエリは trigram では検索できないため LIKE（`prompt LIKE %term%`）にフォールバック。
fn add_search_substring(
    mut query: Select<images::Entity>,
    search_text: &str,
) -> Select<images::Entity> {
    // 1. Extract quoted phrases
    let mut phrases = Vec::new();
    let mut remainder = String::new();

    let mut in_quotes = false;
    let mut current = String::new();

    for c in search_text.chars() {
        match c {
            '"' => {
                if in_quotes {
                    phrases.push(current.clone());
                    current.clear();
                }
                in_quotes = !in_quotes;
            }
            _ => {
                if in_quotes {
                    current.push(c);
                } else {
                    remainder.push(c);
                }
            }
        }
    }

    // 2. Quoted phrases → 部分一致（AND）。
    //    フレーズ全体を正規化し、1つの連続する部分一致語として扱う。
    //    例: "cow boy" → trigram の "cow boy" 部分一致（3文字未満は LIKE）。
    for phrase in &phrases {
        let p = process_prompt(phrase).trim().to_string();
        if !p.is_empty() {
            query = add_substring_filter(query, &p);
        }
    }

    // 3. Remaining terms → 部分一致の OR。
    //    3文字未満の語（LIKE）と 3文字以上の語（trigram FTS）は、
    //    いずれも「いっずれかの語でマッチ」の意味論のため、同じ OR 群にまとめる。
    remainder = process_prompt(&remainder);
    let terms: Vec<&str> = remainder.split_whitespace().collect();
    if !terms.is_empty() {
        let like_terms: Vec<&str> =
            terms.iter().copied().filter(|t| t.chars().count() < 3).collect();
        let fts_terms: Vec<&str> =
            terms.iter().copied().filter(|t| t.chars().count() >= 3).collect();

        let mut cond = Condition::any();

        // 3文字未満の語: LIKE 部分一致（インデックス不要）
        for t in &like_terms {
            let like = format!("%{}%", t);
            cond = cond.add(Expr::col(images::Column::Prompt).like(like));
        }

        // 3文字以上の語: trigram FTS で部分一致（prompt_search 列限定・パラメータ化）
        if !fts_terms.is_empty() {
            let or_terms = fts_terms
                .iter()
                .map(|t| format!("prompt_search:{}", fts_quote(t)))
                .collect::<Vec<_>>()
                .join(" OR ");
            cond = cond.add(Expr::cust_with_expr(
                "images.id IN (SELECT rowid FROM images_fts_trigram WHERE images_fts_trigram MATCH ?)",
                SimpleExpr::value(or_terms),
            ));
        }

        query = query.filter(cond);
    }

    query
}

/// 単一の文字列部分一致フィルタを追加。
/// 3 文字以上は trigram FTS、それ未満は LIKE にフォールバック。
fn add_substring_filter(
    query: Select<images::Entity>,
    term: &str,
) -> Select<images::Entity> {
    if term.chars().count() < 3 {
        let like = format!("%{}%", term);
        return query.filter(Expr::col(images::Column::Prompt).like(like));
    }
    let fts_query = format!("prompt_search:{}", fts_quote(term));
    query.filter(Expr::cust_with_expr(
        "images.id IN (SELECT rowid FROM images_fts_trigram WHERE images_fts_trigram MATCH ?)",
        SimpleExpr::value(fts_query),
    ))
}

/// FTS5 の quoted-string として安全に埋め込めるように、ダブルクォートを二重化して "..." で囲む。
fn fts_quote(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

pub fn process_prompt(prompt: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    let mut prompt = prompt.nfkc().collect::<String>();
    prompt = prompt.to_lowercase();
    prompt = prompt.replace(
        [
            ',', // common prompt separators
            '|', '\n', '\r', '\t', ';', ':',
            // brackets / grouping (usually not meaningful for search)
            '(', ')', '[', ']', '{', '}', // prompt syntax noise
            '<', '>', '=', '+', '*', '~',
            // quotes (normalize to spaces, phrase search uses user input)
            '"', '“', '”', '‘', '’', // slashes
            '/', '\\',
        ],
        " ",
    );

    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_prompt() {
        let prompt = "(masterpiece), woman | cyber-punk portrait";
        let processed = process_prompt(prompt);
        assert_eq!(processed, "masterpiece woman cyber-punk portrait");

        let prompt = "<lora:facefix:1.0> close-up, woman's face";
        let processed = process_prompt(prompt);
        assert_eq!(processed, "lora facefix 1.0 close-up woman's face");
    }

    #[test]
    fn test_fts_quote() {
        assert_eq!(fts_quote("cat"), "\"cat\"");
        // FTS5 の引用文字列ではダブルクォートを二重化してエスケープする
        assert_eq!(fts_quote("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn test_is_ascii_or_latin() {
        // 純 ASCII（英字・数字・記号・空白）は word 側
        assert!(is_ascii_or_latin("skyscraper"));
        assert!(is_ascii_or_latin("12345678"));
        assert!(is_ascii_or_latin("cyber-punk 2.0"));
        assert!(is_ascii_or_latin("!!!"));
        assert!(is_ascii_or_latin(""));

        // ラテン文字（子音付き・拡張）は word 側
        assert!(is_ascii_or_latin("café")); // é: Latin-1
        assert!(is_ascii_or_latin("żółw")); // ż, ł: Latin Extended-A
        assert!(is_ascii_or_latin("phở")); // ợ: Latin Extended Additional（ベトナム語）
        assert!(is_ascii_or_latin("straße")); // ß: Latin-1

        // 非ラテン文字が混在すれば substring 側
        assert!(!is_ascii_or_latin("ネコ")); // かな
        assert!(!is_ascii_or_latin("猫")); // 漢字
        assert!(!is_ascii_or_latin("собака")); // キリル
        assert!(!is_ascii_or_latin("αγορά")); // ギリシャ
        assert!(!is_ascii_or_latin("cat 猫")); // 混在
        assert!(!is_ascii_or_latin("한국어")); // ハングル
    }
}
