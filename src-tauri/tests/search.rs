mod common;

#[cfg(test)]
mod tests {
    use crate::common::*;
    use sea_orm::ConnectionTrait;

    /// list_images を search + search_mode のみで呼ぶヘルパ。
    /// 引数: project_ids, search, search_mode, filters, sort, direction, take, skip,
    ///       count, show_video, show_image, show_disconnected
    ///
    /// show_disconnected は Some(true) を固定する。検索テストは「テキストのマッチング」を
    /// 検証するものであり、watch folder の可視性（is_missing/is_locked）は無関係なため、
    /// サンプル DB の watch folder がテスト間の temp パスの移動で missing に判定される
    /// 競合を避けるために可視性フィルタを常にスキップする。
    macro_rules! run_search {
        ($dtps:expr, $search:expr, $mode:expr) => {{
            $dtps.list_images(
                None, // project_ids
                $search.map(|s| s.to_string()), // search
                $mode.map(|m| m.to_string()), // search_mode
                None, // filters
                None, // sort
                None, // direction
                None, // take
                None, // skip
                None, // count
                None, // show_video
                None, // show_image
                Some(true), // show_disconnected
            )
            .await
            .unwrap()
        }};
    }

    #[tokio::test]
    async fn search_images_simple() {
        // Use the sample db (contains existing images)
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        // Test simple word search
        let result = run_search!(dtps, Some("skyscraper"), Some("word"));
        assert!(
            result.total > 0,
            "Should find at least some images for 'skyscraper'"
        );

        dtps.stop().await;
    }

    #[tokio::test]
    async fn search_images_phrase() {
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        // Test phrase search with quotes
        let result = run_search!(dtps, Some("\"futuristic city\""), Some("word"));
        assert!(
            result.total > 0,
            "Should find results for quoted phrase 'futuristic city'"
        );

        dtps.stop().await;
    }

    #[tokio::test]
    async fn search_images_multiple_terms() {
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        // Test multiple terms (OR search by default in FTS)
        let result = run_search!(dtps, Some("snake skyscraper"), Some("word"));
        // It should find both the skyscraper and the snake images
        assert!(result.total >= 2);

        dtps.stop().await;
    }

    #[tokio::test]
    async fn search_images_no_results() {
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        let result = run_search!(dtps, Some("nonexistent_term_xyz"), Some("word"));
        assert_eq!(result.total, 0);

        dtps.stop().await;
    }

    #[tokio::test]
    async fn search_images_substring_mode() {
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        // Substring mode should also find 'skyscraper' (the term is itself a substring)
        let result = run_search!(dtps, Some("skyscraper"), Some("substring"));
        assert!(
            result.total > 0,
            "Substring mode should find 'skyscraper'"
        );

        // A term that is a substring of a larger word should match in substring mode
        // (e.g. 'crap' is part of 'skyscraper' but would NOT match in word mode).
        let result2 = run_search!(dtps, Some("crap"), Some("substring"));
        assert!(
            result2.total > 0,
            "Substring mode should match 'crap' inside 'skyscraper'"
        );

        dtps.stop().await;
    }
    #[tokio::test]
    async fn search_images_substring_mixed_terms_or() {
        // 混合長クエリ（3文字未満の LIKE 語 + 3文字以上の FTS 語）は OR 結合でなければならない
        // （AND にすると「両方を含む画像だけ」に減ってしまう）。
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        let a = run_search!(dtps, Some("crap"), Some("substring")).total; // FTS 語
        let b = run_search!(dtps, Some("ty"), Some("substring")).total; // LIKE 語
        assert!(a > 0, "'crap' should match via trigram FTS");
        assert!(b > 0, "'ty' should match via LIKE fallback");

        // OR 意味論なら、混合クエリは各単一語の結果以上でなければならない
        let c = run_search!(dtps, Some("crap ty"), Some("substring")).total;
        assert!(
            c >= a.max(b),
            "mixed query must OR its terms (got {} < max({}, {}))",
            c,
            a,
            b
        );

        dtps.stop().await;
    }

    #[tokio::test]
    async fn search_images_substring_short_term_like() {
        // 3文字未満の語は trigram が使えないため LIKE フォールバックでマッチすること
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        let result = run_search!(dtps, Some("ty"), Some("substring"));
        assert!(
            result.total > 0,
            "2-char term 'ty' (part of 'city') should match via LIKE"
        );

        dtps.stop().await;
    }

    #[tokio::test]
    async fn search_images_substring_phrase() {
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        // 引用語は trigram FTS で連続部分一致（prompt_search 列限定）
        let result = run_search!(dtps, Some("\"futuristic city\""), Some("substring"));
        assert!(
            result.total > 0,
            "Quoted phrase 'futuristic city' should match in substring mode"
        );

        dtps.stop().await;
    }

    async fn ready_project_id(dtps: &dtm_lib::dtp_service::DTPService) -> i64 {
        // テスト画像を挿入するため、サンプル DB の既存画像の project_id を使用する。
        // 検索は show_disconnected=true で実行されるため、watch folder の可視性は無関係。
        use entity::images;
        use sea_orm::{EntityTrait, QuerySelect};

        let db = dtps.get_db().await.unwrap();
        let row = images::Entity::find()
            .select_only()
            .column(images::Column::ProjectId)
            .limit(1)
            .into_tuple::<(i64,)>()
            .one(&db.db)
            .await
            .unwrap()
            .expect("sample db should have an image");
        row.0
    }

    #[tokio::test]
    async fn search_images_substring_cjk() {
        let (dtps, _event_helper, _wfh, _db_path) = test_fixture(false, true).await;

        let project_id = ready_project_id(&dtps).await;
        let db = dtps.get_db().await.unwrap();

        // サンプル DB には CJK データがないため、テスト画像を直接挿入する。
        // prompt_search / negative_prompt_search はインポート時と同じく
        // process_prompt 済み（ここでは小文字・正規化済み）の値を格納する。
        let prompt = "ネコとイヌのイラスト";
        db.db
            .execute_unprepared(
                &format!(
                    "INSERT INTO images \
                     (project_id, node_id, preview_id, clip_id, wall_clock, prompt, \
                      negative_prompt, prompt_search, negative_prompt_search, \
                      start_width, start_height, seed, strength, steps, guidance_scale, \
                      shift, sampler, hires_fix, tiled_decoding, tiled_diffusion, \
                      tea_cache, cfg_zero_star, has_mask, has_depth, has_pose, has_color, \
                      has_custom, has_scribble, has_shuffle) \
                     VALUES ({project_id}, 0, 0, 0, '2024-01-01 00:00:00', \
                             '{prompt}', '', '{prompt}', '', 512, 512, 42, 1.0, 20, 7.0, 0.0, \
                             0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)"
                ),
            )
            .await
            .unwrap();

        // 挿入行がインデックスに反映されるよう再構築（trigger なし設計のため rebuild が必要）
        db.db
            .execute_unprepared("INSERT INTO images_fts(images_fts) VALUES('rebuild')")
            .await
            .unwrap();
        db.db
            .execute_unprepared("INSERT INTO images_fts_trigram(images_fts_trigram) VALUES('rebuild')")
            .await
            .unwrap();

        // 3文字以上の CJK 語は trigram FTS で部分一致（word モードでは CJK は分割されないため非マッチ）
        let r1 = run_search!(dtps, Some("イラスト"), Some("substring"));
        assert!(r1.total >= 1, "CJK substring 'イラスト' should match via trigram FTS");

        let r1w = run_search!(dtps, Some("イラスト"), Some("word"));
        assert_eq!(
            r1w.total, 0,
            "word mode must not match CJK substring (porter does not split CJK)"
        );

        // 3文字未満の CJK 語は LIKE フォールバックでマッチ
        let r2 = run_search!(dtps, Some("イヌ"), Some("substring"));
        assert!(r2.total >= 1, "2-char CJK term 'イヌ' should match via LIKE");

        // 連続しない文字列は誤検出しないこと（「ネコ」「コ」の後に「イ」が続かない＝非連続）
        let r3 = run_search!(dtps, Some("ネコイ"), Some("substring"));
        assert_eq!(r3.total, 0, "non-contiguous CJK text must not match");

        dtps.stop().await;
    }


}
