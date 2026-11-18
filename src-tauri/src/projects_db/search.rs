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
/// 全モードで diacritic（子音符号）は無視される（"cafe" は "café" にマッチする）。
/// process_prompt（NFKC → NFD による diacritic 除去 → 小文字化）を
/// コンテンツ側（prompt_search 列）とクエリ側の両方に適用することで実現する。
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

/// diacritic（子音符号）を除去する（NFD 分解 → combining mark 除外 → NFKC 再結合）。
/// prompt_search 列の正規化（process_prompt）と LIKE 経路の diacritic 無視一致に使用する。
/// 例: "café" → "cafe", "naïve" → "naive", "żółw" → "zołw"（ł は NFD 分解できないため残る）
fn strip_diacritics(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    let decomposed: String = text
        .nfd()
        .filter(|&c| !is_combining_mark(c))
        .collect();
    decomposed.nfkc().collect()
}

/// combining mark（結合記号）かどうか。
/// NFD 分解で実際に現れる主要スクリプトの範囲（ラテン・キリル・ヘブライ・アラビア・インド系等）をカバーする。
fn is_combining_mark(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036F   // Combining Diacritical Marks（ラテンの diacritic の大半）
            | 0x0483..=0x0489 // Cyrillic
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7       // Hebrew points
            | 0x0610..=0x061A
            | 0x064B..=0x065F // Arabic
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E8
            | 0x06EA..=0x06ED
            | 0x0900..=0x0903
            | 0x093A..=0x094C
            | 0x094E..=0x094F // Devanagari
            | 0x1AB0..=0x1AFF // Combining Diacritical Marks Extended
            | 0x1DC0..=0x1DFF // Combining Diacritical Marks Supplement
            | 0x20D0..=0x20FF // Combining Diacritical Marks for Symbols
            | 0xFE20..=0xFE2F // Combining Half Marks
    )
}

/// 語マッチモード（既存挙動）。
/// 引用語 `"..."` は正規化後（小文字・diacritic 除去）に prompt_search 列で LIKE の部分一致、
/// 残り語は porter FTS（OR）でマッチ。
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
    //    正規化後（小文字・diacritic 除去）の prompt_search 列で部分一致させることで、
    //    FTS 経路と同様に大文字小文字・アクセントを無視してマッチする
    //    （例: "Café Latte" ≡ "cafe latte"）。
    for phrase in &phrases {
        let p = process_prompt(phrase).trim().to_string();
        if p.is_empty() {
            continue;
        }
        let like = format!("%{}%", p);
        query = query.filter(Expr::col(images::Column::PromptSearch).like(like));
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
/// 3 文字未満のクエリは trigram では検索できないため LIKE（`prompt_search LIKE %term%`、
/// 正規化済みのため diacritic 無視）にフォールバック。
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

        // 3文字未満の語: prompt_search での LIKE 部分一致（インデックス不要、diacritic 無視）
        for t in &like_terms {
            let like = format!("%{}%", t);
            cond = cond.add(Expr::col(images::Column::PromptSearch).like(like));
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
/// 3 文字以上は trigram FTS、それ未満は prompt_search での LIKE（diacritic 無視）にフォールバック。
fn add_substring_filter(
    query: Select<images::Entity>,
    term: &str,
) -> Select<images::Entity> {
    if term.chars().count() < 3 {
        let like = format!("%{}%", term);
        return query.filter(Expr::col(images::Column::PromptSearch).like(like));
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

/// 検索用プロンプトの正規化（NFKC → diacritic 除去 → 小文字 → 区切り記号をスペース化）。
/// prompt_search / negative_prompt_search 列の生成と、検索クエリ側の正規化に使用する。
pub fn process_prompt(prompt: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    let mut prompt = prompt.nfkc().collect::<String>();
    prompt = strip_diacritics(&prompt);
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

        // diacritic は除去され、大文字小文字・区切り記号も正規化される
        let prompt = "Café Latte, (portrait)";
        let processed = process_prompt(prompt);
        assert_eq!(processed, "cafe latte portrait");
    }

    #[test]
    fn test_strip_diacritics() {
        // ラテンの diacritic は除去される
        assert_eq!(strip_diacritics("café"), "cafe");
        assert_eq!(strip_diacritics("naïve"), "naive");
        assert_eq!(strip_diacritics("phở"), "pho"); // ợ = o + horn + dot-below
        // ł は NFD で分解できない（ストロークは combining mark でない）ため残る
        assert_eq!(strip_diacritics("żółw"), "zołw");
        // 非ラテン文字 / 結合記号なし: 不変
        assert_eq!(strip_diacritics("ネコ"), "ネコ");
        assert_eq!(strip_diacritics("straße"), "straße"); // ß は diacritic でない
        assert_eq!(strip_diacritics("ASCII 123"), "ASCII 123");
        assert_eq!(strip_diacritics(""), "");
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
