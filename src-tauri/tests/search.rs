mod common;

#[cfg(test)]
mod tests {
    use crate::common::*;

    /// list_images を search + search_mode のみで呼ぶヘルパ。
    /// 引数: project_ids, search, search_mode, filters, sort, direction, take, skip,
    ///       count, show_video, show_image, show_disconnected
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
                None, // show_disconnected
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
}
