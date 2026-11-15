use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // trigram tokenizer による部分一致用 FTS インデックス
        // 既存の porter 版 images_fts と並行して、CJK（漢字・日本語）の文字列部分一致検索を可能にする。
        // trigram tokenizer は prefix オプション非対応。
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                    CREATE VIRTUAL TABLE IF NOT EXISTS images_fts_trigram
                    USING fts5(
                        prompt_search,
                        negative_prompt_search,
                        content='images',
                        content_rowid='id',
                        tokenize='trigram'
                    );
                "#,
            )
            .await?;

        // 既存データに追従してインデックスを構築
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT INTO images_fts_trigram(images_fts_trigram) VALUES('rebuild')",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS images_fts_trigram;")
            .await?;

        Ok(())
    }
}
