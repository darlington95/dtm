use entity::images;
use sea_orm::{Condition, ExprTrait, QueryFilter, Select};
use sea_query::{Expr, SimpleExpr};

/// 検索モード:
/// - "word"     : 既定。porter トークナイザーの FTS で語マッチ（stemming / prefix）。
///                既存の挙動。CJK（漢字・日本語）の任意位置の部分一致はできない。
/// - "substring": trigram トークナイザーの FTS で文字列の部分一致。
///                漢字・日本語を含む CJK の検索に有効。
pub fn add_search(
    query: Select<images::Entity>,
    search_text: &str,
    mode: &str,
) -> Select<images::Entity> {
    let search_text = search_text.trim();
    if search_text.is_empty() {
        return query;
    }

    if mode == "substring" {
        add_search_substring(query, search_text)
    } else {
        add_search_word(query, search_text)
    }
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
/// 引用語・非引用語いずれも trigram FTS（`images_fts_trigram`）で部分一致。
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
    remainder = process_prompt(&remainder);
    let terms: Vec<&str> = remainder.split_whitespace().collect();
    if !terms.is_empty() {
        let like_terms: Vec<&str> =
            terms.iter().copied().filter(|t| t.chars().count() < 3).collect();
        let fts_terms: Vec<&str> =
            terms.iter().copied().filter(|t| t.chars().count() >= 3).collect();

        // 3文字未満の語: LIKE 部分一致（OR 結合、インデックス不要）
        if !like_terms.is_empty() {
            let mut cond = Condition::any();
            for t in &like_terms {
                let like = format!("%{}%", t);
                cond = cond.add(Expr::col(images::Column::Prompt).like(like));
            }
            query = query.filter(cond);
        }

        // 3文字以上の語: trigram FTS で部分一致（OR 結合）
        if !fts_terms.is_empty() {
            let or_terms = fts_terms
                .iter()
                .map(|t| fts_quote(t))
                .collect::<Vec<_>>()
                .join(" OR ");
            let subq = format!(
                "images.id IN (SELECT rowid FROM images_fts_trigram WHERE images_fts_trigram MATCH '{or_terms}')"
            );
            query = query.filter(Expr::cust(subq));
        }
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
    let subq = format!(
        "images.id IN (SELECT rowid FROM images_fts_trigram WHERE images_fts_trigram MATCH 'prompt_search:\"{}\"')",
        fts_quote(term),
    );
    query.filter(Expr::cust(subq))
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
}
