use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

/// diacritic（子音符号）を除去する（NFD 分解 → combining mark 除外 → NFKC 再結合）。
/// メイン crate（projects_db/search.rs）の同名関数をミラーしている。
/// ここでは既存行の prompt_search / negative_prompt_search のバックフィルにのみ使用する。
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

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let backend = db.get_database_backend();

        // 1) バックフィル: 既存行の prompt_search / negative_prompt_search から diacritic を除去する。
        //    （メイン crate の process_prompt 変更後にインポートされた行は既にこの形式なので、
        //    値が変化しない行は UPDATE しない。）
        //    FTS の再構築前に実行することで、新しいインデックスが除去済み値をインデックスする。
        const BATCH: i64 = 1000;
        let mut last_id: i64 = 0;
        loop {
            let rows = db
                .query_all_raw(Statement::from_sql_and_values(
                    backend,
                    "SELECT id, prompt_search, negative_prompt_search \
                     FROM images WHERE id > ? ORDER BY id LIMIT ?",
                    [Value::from(last_id), Value::from(BATCH)],
                ))
                .await?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                let id = row.try_get::<i64>("", "id")?;
                let prompt_search = row.try_get::<String>("", "prompt_search")?;
                let negative_prompt_search =
                    row.try_get::<String>("", "negative_prompt_search")?;

                let new_prompt = strip_diacritics(&prompt_search);
                let new_negative = strip_diacritics(&negative_prompt_search);
                if new_prompt != prompt_search || new_negative != negative_prompt_search {
                    db.execute_raw(Statement::from_sql_and_values(
                        backend,
                        "UPDATE images SET prompt_search = ?, negative_prompt_search = ? \
                         WHERE id = ?",
                        [
                            Value::from(new_prompt),
                            Value::from(new_negative),
                            Value::from(id),
                        ],
                    ))
                    .await?;
                }
            }
            last_id = rows.last().unwrap().try_get::<i64>("", "id")?;
        }

        // 2) 両 FTS インデックスを再構築する（バックフィル後の除去済み値をインデックスに反映）。
        //    トークナイザーは変更しない（vendored SQLite 3.51 の trigram は
        //    remove_diacritics mode 2 非対応のため）。diacritic 除去は
        //    process_prompt（NFKC → NFD 除去 → 小文字化）でコンテンツ側・クエリ側の
        //    両方に一貫して適用されるため、トークナイザー側の除去は不要である。
        db.execute_unprepared("INSERT INTO images_fts(images_fts) VALUES('rebuild')")
            .await?;
        db.execute_unprepared("INSERT INTO images_fts_trigram(images_fts_trigram) VALUES('rebuild')")
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // 注意: up のステップ 1（diacritic 除去のバックフィル）は元に戻せないため、
        // FTS の再構築も巻き戻す意味がなく、down は何もしない。
        Ok(())
    }
}

