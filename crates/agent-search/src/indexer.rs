//! Search index management using Tantivy.
//!
//! Indexes [`TerminalCommand`] and [`TerminalSession`] documents and provides
//! ranked search with optional structured filters via [`SearchQuery`].

use crate::query::SearchQuery;
use crate::SearchResult;
use agent_core::terminal_session::{TerminalCommand, TerminalSession};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::*;
use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyDocument, Term};
use uuid::Uuid;

/// A single search hit returned by [`SearchIndexer::search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Tantivy relevance score.
    pub score: f32,
    /// The raw command text.
    pub command: String,
    /// Session that owns this command.
    pub session_id: Uuid,
    /// When the command was executed.
    pub timestamp: DateTime<Utc>,
    /// First N characters of output (if stored).
    pub snippet: String,
    /// The program that was run (e.g. `git`, `cargo`).
    pub program: String,
    /// Tool category (e.g. `VersionControl`).
    pub category: String,
    /// Working directory at the time of execution.
    pub directory: String,
    /// Exit code, if the command completed.
    pub exit_code: Option<i32>,
}

/// Full-text search indexer backed by Tantivy.
pub struct SearchIndexer {
    index: Index,
    #[allow(dead_code)] // retained for schema introspection / future API
    schema: Schema,
    // -- stored field handles --
    f_id: Field,
    f_session_id: Field,
    f_command: Field,
    f_program: Field,
    f_category: Field,
    f_directory: Field,
    f_timestamp: Field,
    f_exit_code: Field,
    f_output_snippet: Field,
}

impl SearchIndexer {
    // ---- construction helpers (shared schema) ----

    fn build_schema() -> (Schema, FieldHandles) {
        let mut b = Schema::builder();
        let h = FieldHandles {
            id: b.add_text_field("id", STRING | STORED),
            session_id: b.add_text_field("session_id", STRING | STORED),
            command: b.add_text_field("command", TEXT | STORED),
            program: b.add_text_field("program", STRING | STORED),
            category: b.add_text_field("category", STRING | STORED),
            directory: b.add_text_field("directory", TEXT | STORED),
            timestamp: b.add_i64_field("timestamp", INDEXED | STORED),
            exit_code: b.add_i64_field("exit_code", INDEXED | STORED),
            output_snippet: b.add_text_field("output_snippet", TEXT | STORED),
        };
        (b.build(), h)
    }

    fn from_index(index: Index, schema: Schema, h: FieldHandles) -> Self {
        Self {
            index,
            schema,
            f_id: h.id,
            f_session_id: h.session_id,
            f_command: h.command,
            f_program: h.program,
            f_category: h.category,
            f_directory: h.directory,
            f_timestamp: h.timestamp,
            f_exit_code: h.exit_code,
            f_output_snippet: h.output_snippet,
        }
    }

    /// Open (or create) a persistent index at `path`.
    pub fn open(path: &Path) -> SearchResult<Self> {
        std::fs::create_dir_all(path)?;
        let (schema, h) = Self::build_schema();

        let index = if path.join("meta.json").exists() {
            Index::open_in_dir(path)?
        } else {
            Index::create_in_dir(path, schema.clone())?
        };

        Ok(Self::from_index(index, schema, h))
    }

    /// Create a RAM-only index (useful for tests).
    pub fn in_memory() -> SearchResult<Self> {
        let (schema, h) = Self::build_schema();
        let index = Index::create_in_ram(schema.clone());
        Ok(Self::from_index(index, schema, h))
    }

    /// Obtain a writer with a 50 MB heap budget.
    pub fn writer(&self) -> SearchResult<IndexWriter> {
        Ok(self.index.writer(50_000_000)?)
    }

    // ---- indexing ----

    /// Index a single [`TerminalCommand`].
    pub fn index_command(
        &self,
        writer: &IndexWriter,
        cmd: &TerminalCommand,
    ) -> SearchResult<()> {
        let mut doc = TantivyDocument::default();
        doc.add_text(self.f_id, cmd.id.to_string());
        doc.add_text(self.f_session_id, cmd.session_id.to_string());
        doc.add_text(self.f_command, &cmd.command_text);
        doc.add_text(
            self.f_directory,
            cmd.working_directory.to_string_lossy().as_ref(),
        );
        doc.add_i64(self.f_timestamp, cmd.started_at.timestamp());

        // Parsed metadata (program / category).
        if let Some(ref parsed) = cmd.parsed {
            doc.add_text(self.f_program, &parsed.program);
            doc.add_text(self.f_category, format!("{:?}", parsed.tool_category));
        } else {
            doc.add_text(self.f_program, "");
            doc.add_text(self.f_category, "Other");
        }

        if let Some(code) = cmd.exit_code {
            doc.add_i64(self.f_exit_code, code as i64);
        } else {
            // Sentinel value so field is always present.
            doc.add_i64(self.f_exit_code, i64::MIN);
        }

        doc.add_text(self.f_output_snippet, "");
        writer.add_document(doc)?;
        Ok(())
    }

    /// Index every command in a [`TerminalSession`] given a command list.
    ///
    /// The session itself is not stored as a separate document; all its
    /// metadata is denormalized onto its commands.
    pub fn index_session(
        &self,
        writer: &IndexWriter,
        _session: &TerminalSession,
        commands: &[TerminalCommand],
    ) -> SearchResult<()> {
        for cmd in commands {
            self.index_command(writer, cmd)?;
        }
        Ok(())
    }

    /// Delete every document belonging to a session.
    pub fn delete_session(
        &self,
        writer: &IndexWriter,
        session_id: &Uuid,
    ) -> SearchResult<()> {
        let term = Term::from_field_text(self.f_session_id, &session_id.to_string());
        writer.delete_term(term);
        Ok(())
    }

    // ---- querying ----

    /// Run a [`SearchQuery`] and return at most `limit` hits, ranked by score.
    pub fn search(&self, query: &SearchQuery, limit: usize) -> SearchResult<Vec<SearchHit>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let searcher = reader.searcher();

        let tantivy_query = self.build_query(query)?;
        let top_docs = searcher.search(&*tantivy_query, &TopDocs::with_limit(limit))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr)?;
            hits.push(self.doc_to_hit(score, &doc));
        }
        Ok(hits)
    }

    // ---- internal helpers ----

    /// Build a composite Tantivy query from a [`SearchQuery`].
    fn build_query(&self, q: &SearchQuery) -> SearchResult<Box<dyn Query>> {
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Free-text across command + directory + output_snippet.
        if !q.text.is_empty() {
            let parser = QueryParser::for_index(
                &self.index,
                vec![self.f_command, self.f_directory, self.f_output_snippet],
            );
            let tq = parser.parse_query(&q.text)?;
            clauses.push((Occur::Must, tq));
        }

        // Program filter (exact match).
        if let Some(ref program) = q.program {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.f_program, program),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        // Category filter (exact match on Debug string, e.g. "VersionControl").
        if let Some(ref category) = q.category {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.f_category, category),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        // Directory filter (text search within directory field).
        if let Some(ref dir) = q.directory {
            let parser = QueryParser::for_index(&self.index, vec![self.f_directory]);
            let dq = parser.parse_query(dir)?;
            clauses.push((Occur::Must, dq));
        }

        // Date range.
        if let Some(after) = q.after {
            clauses.push((
                Occur::Must,
                Box::new(RangeQuery::new_i64("timestamp".to_string(), after.timestamp()..i64::MAX)),
            ));
        }
        if let Some(before) = q.before {
            clauses.push((
                Occur::Must,
                Box::new(RangeQuery::new_i64("timestamp".to_string(), i64::MIN..before.timestamp() + 1)),
            ));
        }

        // Exit code filter.
        if let Some(code) = q.exit_code {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_i64(self.f_exit_code, code as i64),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if clauses.is_empty() {
            // No filters at all -- return everything.
            Ok(Box::new(AllQuery))
        } else {
            Ok(Box::new(BooleanQuery::new(clauses)))
        }
    }

    /// Convert a retrieved Tantivy document into a [`SearchHit`].
    fn doc_to_hit(&self, score: f32, doc: &TantivyDocument) -> SearchHit {
        let text = |f: Field| -> String {
            doc.get_first(f)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        let ts_secs = doc
            .get_first(self.f_timestamp)
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let exit_raw = doc
            .get_first(self.f_exit_code)
            .and_then(|v| v.as_i64())
            .unwrap_or(i64::MIN);

        let exit_code = if exit_raw == i64::MIN {
            None
        } else {
            Some(exit_raw as i32)
        };

        let session_id = text(self.f_session_id)
            .parse::<Uuid>()
            .unwrap_or(Uuid::nil());

        SearchHit {
            score,
            command: text(self.f_command),
            session_id,
            timestamp: Utc
                .timestamp_opt(ts_secs, 0)
                .single()
                .unwrap_or_else(Utc::now),
            snippet: text(self.f_output_snippet),
            program: text(self.f_program),
            category: text(self.f_category),
            directory: text(self.f_directory),
            exit_code,
        }
    }
}

/// Internal helper to shuttle field handles around during construction.
struct FieldHandles {
    id: Field,
    session_id: Field,
    command: Field,
    program: Field,
    category: Field,
    directory: Field,
    timestamp: Field,
    exit_code: Field,
    output_snippet: Field,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::command_parser::CommandParser;
    use chrono::Duration;
    use std::path::PathBuf;

    /// Helper: create a TerminalCommand with parsed metadata.
    fn make_cmd(
        session_id: Uuid,
        text: &str,
        dir: &str,
        exit_code: Option<i32>,
        ts: DateTime<Utc>,
    ) -> TerminalCommand {
        let parser = CommandParser::new();
        let mut cmd = TerminalCommand::new(
            Uuid::new_v4(),
            session_id,
            1,
            text.to_string(),
            PathBuf::from(dir),
            ts,
        );
        cmd.parsed = parser.parse(text);
        if let Some(code) = exit_code {
            cmd.complete(code, ts + Duration::milliseconds(100));
        }
        cmd
    }

    fn make_session(id: Uuid) -> TerminalSession {
        TerminalSession::new(
            id,
            "bash".into(),
            PathBuf::from("/home/user"),
            None,
            Utc::now(),
        )
    }

    #[test]
    fn test_in_memory_index_and_search() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();

        let cmd1 = make_cmd(sid, "cargo test --lib", "/home/user/project", Some(0), Utc::now());
        let cmd2 = make_cmd(sid, "cargo build --release", "/home/user/project", Some(0), Utc::now());
        let cmd3 = make_cmd(sid, "git status", "/home/user/project", Some(0), Utc::now());

        idx.index_command(&writer, &cmd1).unwrap();
        idx.index_command(&writer, &cmd2).unwrap();
        idx.index_command(&writer, &cmd3).unwrap();
        writer.commit().unwrap();

        // Search for "cargo"
        let q = SearchQuery { text: "cargo".into(), ..Default::default() };
        let hits = idx.search(&q, 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.command.contains("cargo")));
    }

    #[test]
    fn test_search_with_program_filter() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();

        idx.index_command(&writer, &make_cmd(sid, "git status", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "git log", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "cargo build", "/tmp", Some(0), Utc::now())).unwrap();
        writer.commit().unwrap();

        let q = SearchQuery { program: Some("git".into()), ..Default::default() };
        let hits = idx.search(&q, 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.program == "git"));
    }

    #[test]
    fn test_search_with_category_filter() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();

        idx.index_command(&writer, &make_cmd(sid, "git commit -m 'wip'", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "cargo test", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "docker ps", "/tmp", Some(0), Utc::now())).unwrap();
        writer.commit().unwrap();

        let q = SearchQuery {
            category: Some("VersionControl".into()),
            ..Default::default()
        };
        let hits = idx.search(&q, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].program, "git");
    }

    #[test]
    fn test_search_with_date_filter() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();

        let old = Utc::now() - Duration::days(30);
        let recent = Utc::now() - Duration::hours(1);

        idx.index_command(&writer, &make_cmd(sid, "old command", "/tmp", Some(0), old)).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "recent command", "/tmp", Some(0), recent)).unwrap();
        writer.commit().unwrap();

        let q = SearchQuery {
            after: Some(Utc::now() - Duration::days(1)),
            ..Default::default()
        };
        let hits = idx.search(&q, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].command.contains("recent"));
    }

    #[test]
    fn test_delete_session() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid1 = Uuid::new_v4();
        let sid2 = Uuid::new_v4();

        idx.index_command(&writer, &make_cmd(sid1, "ls -la", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid1, "pwd", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid2, "whoami", "/tmp", Some(0), Utc::now())).unwrap();
        writer.commit().unwrap();

        // All three present.
        let all = idx.search(&SearchQuery::default(), 10).unwrap();
        assert_eq!(all.len(), 3);

        // Delete session 1.
        idx.delete_session(&writer, &sid1).unwrap();
        writer.commit().unwrap();

        let remaining = idx.search(&SearchQuery::default(), 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].session_id, sid2);
    }

    #[test]
    fn test_empty_query_returns_all() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();

        idx.index_command(&writer, &make_cmd(sid, "echo hello", "/a", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "echo world", "/b", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "echo foo", "/c", Some(0), Utc::now())).unwrap();
        writer.commit().unwrap();

        let hits = idx.search(&SearchQuery::default(), 100).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn test_complex_query_multiple_filters() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();
        let now = Utc::now();

        // This one matches all filters.
        idx.index_command(&writer, &make_cmd(sid, "git commit -m 'fix'", "/home/user/myapp", Some(0), now)).unwrap();
        // Wrong exit code.
        idx.index_command(&writer, &make_cmd(sid, "git push origin main", "/home/user/myapp", Some(1), now)).unwrap();
        // Wrong program.
        idx.index_command(&writer, &make_cmd(sid, "cargo test", "/home/user/myapp", Some(0), now)).unwrap();

        writer.commit().unwrap();

        let q = SearchQuery {
            text: "".into(),
            program: Some("git".into()),
            exit_code: Some(0),
            ..Default::default()
        };
        let hits = idx.search(&q, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].command.contains("commit"));
    }

    #[test]
    fn test_index_session_with_commands() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();
        let session = make_session(sid);

        let cmds = vec![
            make_cmd(sid, "ls", "/tmp", Some(0), Utc::now()),
            make_cmd(sid, "pwd", "/tmp", Some(0), Utc::now()),
        ];

        idx.index_session(&writer, &session, &cmds).unwrap();
        writer.commit().unwrap();

        let hits = idx.search(&SearchQuery::default(), 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.session_id == sid));
    }

    #[test]
    fn test_exit_code_filter() {
        let idx = SearchIndexer::in_memory().unwrap();
        let mut writer = idx.writer().unwrap();
        let sid = Uuid::new_v4();

        idx.index_command(&writer, &make_cmd(sid, "true", "/tmp", Some(0), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "false", "/tmp", Some(1), Utc::now())).unwrap();
        idx.index_command(&writer, &make_cmd(sid, "exit 2", "/tmp", Some(2), Utc::now())).unwrap();
        writer.commit().unwrap();

        let q = SearchQuery { exit_code: Some(1), ..Default::default() };
        let hits = idx.search(&q, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].exit_code, Some(1));
    }

    #[test]
    fn test_persistent_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search-idx");

        // Create and populate.
        {
            let idx = SearchIndexer::open(&path).unwrap();
            let mut w = idx.writer().unwrap();
            let sid = Uuid::new_v4();
            idx.index_command(&w, &make_cmd(sid, "cargo check", "/proj", Some(0), Utc::now())).unwrap();
            w.commit().unwrap();
        }

        // Re-open and verify.
        {
            let idx = SearchIndexer::open(&path).unwrap();
            let hits = idx.search(&SearchQuery::default(), 10).unwrap();
            assert_eq!(hits.len(), 1);
            assert!(hits[0].command.contains("cargo"));
        }
    }
}
