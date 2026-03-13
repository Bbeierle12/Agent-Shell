type FilterFn<T> = Box<dyn Fn(&T) -> bool>;
type SortFn<T> = Box<dyn Fn(&T, &T) -> std::cmp::Ordering>;
/// Extractor for text_search: returns an optional string reference from a record.
pub type TextExtractor<T> = Box<dyn Fn(&T) -> Option<&str>>;

/// In-memory predicate/filter builder for scanning collections.
/// All predicates are ANDed together. For <10K records this is sub-millisecond.
pub struct Query<T> {
    filters: Vec<FilterFn<T>>,
    sorter: Option<SortFn<T>>,
    limit: Option<usize>,
    offset: usize,
}

impl<T: 'static> Query<T> {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            sorter: None,
            limit: None,
            offset: 0,
        }
    }

    /// Add an arbitrary predicate filter.
    pub fn filter(mut self, pred: impl Fn(&T) -> bool + 'static) -> Self {
        self.filters.push(Box::new(pred));
        self
    }

    /// Case-insensitive substring match on a string field.
    pub fn contains(
        self,
        extractor: impl Fn(&T) -> &str + 'static,
        needle: impl Into<String>,
    ) -> Self {
        let needle = needle.into().to_lowercase();
        self.filter(move |item| extractor(item).to_lowercase().contains(&needle))
    }

    /// Case-insensitive substring match on an Option<String> field.
    pub fn contains_opt(
        self,
        extractor: impl Fn(&T) -> &Option<String> + 'static,
        needle: impl Into<String>,
    ) -> Self {
        let needle = needle.into().to_lowercase();
        self.filter(move |item| {
            extractor(item)
                .as_ref()
                .map(|s| s.to_lowercase().contains(&needle))
                .unwrap_or(false)
        })
    }

    /// Multi-field OR text search: matches if any extractor's output contains the needle.
    pub fn text_search(self, extractors: Vec<TextExtractor<T>>, needle: impl Into<String>) -> Self {
        let needle = needle.into().to_lowercase();
        self.filter(move |item| {
            extractors.iter().any(|ext| {
                ext(item)
                    .map(|s| s.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
        })
    }

    /// Sort results.
    pub fn order_by(mut self, cmp: impl Fn(&T, &T) -> std::cmp::Ordering + 'static) -> Self {
        self.sorter = Some(Box::new(cmp));
        self
    }

    /// Limit result count.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Skip first N results.
    pub fn offset(mut self, n: usize) -> Self {
        self.offset = n;
        self
    }

    /// Execute the query against a slice of records.
    pub fn execute<'a>(&self, records: &'a [T]) -> Vec<&'a T> {
        let mut results: Vec<&T> = records
            .iter()
            .filter(|item| self.filters.iter().all(|f| f(item)))
            .collect();

        if let Some(ref sorter) = self.sorter {
            results.sort_by(|a, b| sorter(a, b));
        }

        let results = results.into_iter().skip(self.offset);

        match self.limit {
            Some(n) => results.take(n).collect(),
            None => results.collect(),
        }
    }
}

impl<T: 'static> Default for Query<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct Item {
        name: String,
        description: Option<String>,
        score: i32,
    }

    fn sample_data() -> Vec<Item> {
        vec![
            Item {
                name: "Alpha".into(),
                description: Some("The first item".into()),
                score: 10,
            },
            Item {
                name: "Beta".into(),
                description: Some("Async runtime".into()),
                score: 50,
            },
            Item {
                name: "Gamma".into(),
                description: None,
                score: 30,
            },
            Item {
                name: "Delta Async".into(),
                description: Some("Another async thing".into()),
                score: 20,
            },
        ]
    }

    #[test]
    fn filter_by_predicate() {
        let data = sample_data();
        let results = Query::new().filter(|i: &Item| i.score > 25).execute(&data);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn contains_case_insensitive() {
        let data = sample_data();
        let results = Query::new()
            .contains(|i: &Item| &i.name, "ALPHA")
            .execute(&data);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Alpha");
    }

    #[test]
    fn text_search_multi_field() {
        let data = sample_data();
        let results = Query::new()
            .text_search(
                vec![
                    Box::new(|i: &Item| Some(i.name.as_str())),
                    Box::new(|i: &Item| i.description.as_deref()),
                ],
                "async",
            )
            .execute(&data);
        // "Beta" has "async" in description, "Delta Async" has it in name and description
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn order_by_score() {
        let data = sample_data();
        let results = Query::new()
            .order_by(|a: &Item, b: &Item| b.score.cmp(&a.score))
            .execute(&data);
        assert_eq!(results[0].name, "Beta");
        assert_eq!(results[1].name, "Gamma");
    }

    #[test]
    fn limit_and_offset() {
        let data = sample_data();
        let results = Query::new()
            .order_by(|a: &Item, b: &Item| a.score.cmp(&b.score))
            .offset(1)
            .limit(2)
            .execute(&data);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].score, 20); // Delta (skipped Alpha=10)
        assert_eq!(results[1].score, 30); // Gamma
    }

    #[test]
    fn empty_query_returns_all() {
        let data = sample_data();
        let results = Query::<Item>::new().execute(&data);
        assert_eq!(results.len(), 4);
    }
}
