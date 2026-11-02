//! Hybrid in-memory index, persistence (`records.ndjson`, `meta.json`), and GitHub release tarball support.

use crate::config::ResolvedConfig;
use crate::ingest::{compute_source_fingerprint, ingest_bevy_rui};
use crate::types::{
    IndexMetadata, RegistryData, RegistryEntry, ReleaseManifest, RuiRecord, SearchHit,
    CACHE_VERSION,
};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// In-memory index of `bevy_rui` records with exact-symbol and token-weighted conceptual search.
pub struct RuiIndex {
    pub records: Vec<RuiRecord>,
    pub meta: IndexMetadata,
    /// Exact normalized symbol lookup (`norm_key -> Vec<record_idx>`).
    pub symbols: HashMap<String, Vec<usize>>,
    /// Inverted index over split camelCase/snake_case name tokens (`token -> Vec<record_idx>`).
    pub name_tokens: HashMap<String, Vec<usize>>,
    /// Inverted index over documentation/signature/field tokens (`token -> Vec<record_idx>`).
    pub doc_tokens: HashMap<String, Vec<usize>>,
    /// Reverse doc-link graph: normalized link target (`norm_key`) -> `Vec<record_idx>` of records whose
    /// `doc_links` mention it. `rui_api` uses it to report which indexed symbols reference a lookup,
    /// which makes the doc cross-references in `bevy_rui` verifiable instead of prose-only.
    pub linked_by: HashMap<String, Vec<usize>>,
}

pub fn norm_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        if ch == '<' {
            depth += 1;
        } else if ch == '>' {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && !ch.is_whitespace() {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.trim_start_matches("::")
        .trim_end_matches("()")
        .to_string()
}

/// Splits camelCase and snake_case identifiers into lowercase singular search tokens.
pub fn tokenize_identifier(ident: &str) -> Vec<String> {
    let base = ident.split('<').next().unwrap_or(ident);
    let mut spaced = String::with_capacity(base.len() + 8);
    let chars: Vec<char> = base.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        if i > 0 {
            let prev = chars[i - 1];
            if (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && c.is_ascii_uppercase() {
                spaced.push(' ');
            } else if i + 1 < chars.len()
                && prev.is_ascii_uppercase()
                && c.is_ascii_uppercase()
                && chars[i + 1].is_ascii_lowercase()
            {
                spaced.push(' ');
            }
        }
        if c.is_ascii_alphanumeric() {
            spaced.push(c.to_ascii_lowercase());
        } else {
            spaced.push(' ');
        }
    }

    spaced
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(singularize)
        .collect()
}

pub fn tokenize_prose(text: &str) -> Vec<String> {
    let stopwords = [
        "a", "an", "the", "and", "or", "of", "for", "to", "in", "on", "at", "by", "with", "from",
        "into", "as", "is", "are", "be", "how", "what", "which", "when", "where", "can", "use",
        "using", "pub", "fn", "mut", "impl", "fnonce", "self", "let", "default",
    ];
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if raw.len() < 2 {
            continue;
        }
        for sub in tokenize_identifier(raw) {
            if sub.len() >= 2 && !stopwords.contains(&sub.as_str()) {
                out.push(sub);
            }
        }
    }
    out
}

fn singularize(term: &str) -> String {
    if term.ends_with("ies") && term.len() > 4 {
        format!("{}y", &term[..term.len() - 3])
    } else if term.ends_with("s")
        && term.len() > 3
        && !term.ends_with("ss")
        && !term.ends_with("us")
        && !term.ends_with("tabs")
    {
        term[..term.len() - 1].to_string()
    } else {
        term.to_string()
    }
}

fn iso_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

impl RuiIndex {
    /// Builds an in-memory `RuiIndex` from a list of `RuiRecord`s and metadata.
    pub fn from_records(records: Vec<RuiRecord>, mut meta: IndexMetadata) -> Self {
        let mut symbols: HashMap<String, Vec<usize>> = HashMap::new();
        let mut name_tokens: HashMap<String, Vec<usize>> = HashMap::new();
        let mut doc_tokens: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
        let mut widgets_set: BTreeSet<String> = BTreeSet::new();
        let mut linked_by: HashMap<String, Vec<usize>> = HashMap::new();
        let mut total_doc_links: usize = 0;
        let mut total_implements: usize = 0;

        for (idx, rec) in records.iter().enumerate() {
            *by_kind.entry(rec.kind.clone()).or_insert(0) += 1;
            if let Some(w) = &rec.widget {
                widgets_set.insert(w.clone());
            }

            // Register symbol keys: bare name, owner::name, full_path, and without `bevy_rui::`
            let mut keys = vec![norm_key(&rec.name), norm_key(&rec.full_path)];
            if let Some(owner) = &rec.owner {
                keys.push(norm_key(&format!("{owner}::{}", rec.name)));
            }
            if let Some(stripped) = rec.full_path.strip_prefix("bevy_rui::") {
                keys.push(norm_key(stripped));
            }
            if let Some(stripped) = rec.full_path.strip_prefix("bevy_rui::widgets::") {
                keys.push(norm_key(stripped));
            }
            keys.sort();
            keys.dedup();
            for k in keys {
                if !k.is_empty() {
                    symbols.entry(k).or_default().push(idx);
                }
            }

            // Index name tokens (including widget name if present)
            let mut n_toks = tokenize_identifier(&rec.name);
            if let Some(w) = &rec.widget {
                n_toks.extend(tokenize_identifier(w));
            }
            n_toks.sort();
            n_toks.dedup();
            for tok in n_toks {
                name_tokens.entry(tok).or_default().push(idx);
            }

            // Index doc + signature + field tokens, plus the trait names and normalized doc-link
            // targets so `rui_search` can find a type from a trait it implements (`plugin widget`)
            // or from a symbol its documentation links to.
            let mut body = format!("{} {} {}", rec.signature, rec.docs, rec.module);
            for t in &rec.implements {
                body.push(' ');
                body.push_str(t);
            }
            for l in &rec.doc_links {
                body.push(' ');
                body.push_str(l);
            }
            total_doc_links += rec.doc_links.len();
            total_implements += rec.implements.len();
            for f in &rec.fields {
                body.push(' ');
                body.push_str(&f.name);
                body.push(' ');
                body.push_str(&f.ty);
                body.push(' ');
                body.push_str(&f.docs);
            }
            let mut d_toks = tokenize_prose(&body);
            d_toks.sort();
            d_toks.dedup();
            for tok in d_toks {
                doc_tokens.entry(tok).or_default().push(idx);
            }

            // Reverse doc-link edges: `linked_by[normalized_target]` collects the records whose
            // documentation links to that target, so `rui_api` can answer "what references this?".
            for target in &rec.doc_links {
                linked_by.entry(norm_key(target)).or_default().push(idx);
            }
        }

        meta.total_records = records.len();
        meta.total_symbols = symbols.len();
        meta.by_kind = by_kind;
        meta.widgets = widgets_set.into_iter().collect();
        meta.total_doc_links = total_doc_links;
        meta.total_implements = total_implements;

        Self {
            records,
            meta,
            symbols,
            name_tokens,
            doc_tokens,
            linked_by,
        }
    }

    /// Resolves an exact symbol name or qualified path (e.g. `spawn_button`, `RuiBuilderExt::button`, `RuiTheme`).
    pub fn lookup_symbol(&self, symbol: &str) -> Vec<&RuiRecord> {
        let key = norm_key(symbol);
        if key.is_empty() {
            return Vec::new();
        }
        if let Some(indices) = self.symbols.get(&key) {
            let mut out: Vec<&RuiRecord> = indices.iter().map(|&i| &self.records[i]).collect();
            out.sort_by_key(|r| kind_rank(&r.kind));
            return out;
        }

        // Fallback: suffix match on `::symbol` or substring match on symbol name
        let suffix = format!("::{key}");
        let mut fallback: Vec<&RuiRecord> = self
            .records
            .iter()
            .filter(|r| {
                norm_key(&r.full_path).ends_with(&suffix) || norm_key(&r.name).contains(&key)
            })
            .collect();
        fallback.sort_by_key(|r| (kind_rank(&r.kind), r.name.len()));
        fallback.truncate(10);
        fallback
    }

    /// Performs a hybrid search combining exact symbol matches, identifier name token matches, and full-text doc matches.
    pub fn search(
        &self,
        query: &str,
        kind_filter: Option<&str>,
        widget_filter: Option<&str>,
        limit: usize,
    ) -> Vec<SearchHit<'_>> {
        let mut scores: HashMap<usize, f64> = HashMap::new();
        // Track which field produced the score so the formatter can explain the ranking.
        let mut hit_symbol: HashMap<usize, bool> = HashMap::new();
        let mut hit_name: HashMap<usize, bool> = HashMap::new();
        let mut hit_doc: HashMap<usize, bool> = HashMap::new();

        // 1. Exact symbol match boost
        let q_key = norm_key(query);
        if let Some(exact_indices) = self.symbols.get(&q_key) {
            for &idx in exact_indices {
                *scores.entry(idx).or_insert(0.0) += 100.0;
                hit_symbol.insert(idx, true);
            }
        }

        // 2. Token-based scoring
        let q_tokens = tokenize_prose(query);
        for tok in &q_tokens {
            if let Some(name_hits) = self.name_tokens.get(tok) {
                for &idx in name_hits {
                    *scores.entry(idx).or_insert(0.0) += 18.0;
                    hit_name.insert(idx, true);
                }
            }
            // Prefix match on name tokens
            if tok.len() >= 3 {
                for (n_tok, indices) in &self.name_tokens {
                    if n_tok != tok && (n_tok.starts_with(tok) || tok.starts_with(n_tok)) {
                        for &idx in indices {
                            *scores.entry(idx).or_insert(0.0) += 8.0;
                            hit_name.insert(idx, true);
                        }
                    }
                }
            }
            if let Some(doc_hits) = self.doc_tokens.get(tok) {
                for &idx in doc_hits {
                    *scores.entry(idx).or_insert(0.0) += 4.0;
                    hit_doc.insert(idx, true);
                }
            }
        }

        // 3. Apply kind/widget filters and kind weights
        let mut hits: Vec<SearchHit<'_>> = scores
            .into_iter()
            .filter_map(|(idx, raw_score)| {
                let rec = &self.records[idx];
                if let Some(kf) = kind_filter {
                    if !kf.is_empty() && kf != "all" && rec.kind != kf {
                        return None;
                    }
                }
                if let Some(wf) = widget_filter {
                    if !wf.is_empty() && rec.widget.as_deref() != Some(wf) {
                        return None;
                    }
                }
                let weight = match rec.kind.as_str() {
                    "struct" | "enum" | "trait" => 1.25,
                    "fn" | "trait_method" => 1.2,
                    "module" => 1.1,
                    "method" => 1.0,
                    "example" => 0.95,
                    _ => 0.85,
                };
                let from_symbol = hit_symbol.contains_key(&idx);
                let from_name = hit_name.contains_key(&idx);
                let from_doc = hit_doc.contains_key(&idx);
                let reason = match (from_symbol, from_name, from_doc) {
                    (true, true, true) => "exact symbol + name + docs".to_string(),
                    (true, true, false) => "exact symbol + name".to_string(),
                    (true, false, true) => "exact symbol + docs".to_string(),
                    (true, false, false) => "exact symbol".to_string(),
                    (false, true, true) => "name + docs".to_string(),
                    (false, true, false) => "name".to_string(),
                    (false, false, true) => "docs".to_string(),
                    (false, false, false) => "kind".to_string(),
                };
                Some(SearchHit {
                    record: rec,
                    score: raw_score * weight,
                    reason,
                })
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.record.name.cmp(&b.record.name))
        });
        hits.truncate(limit);
        hits
    }

    /// Returns all records belonging to a specific widget module (e.g. `"button"`, `"docks"`, `"code_editor"`).
    pub fn widget_records(&self, widget_name: &str) -> Vec<&RuiRecord> {
        let target = widget_name.trim().to_ascii_lowercase().replace('-', "_");
        let singular = singularize(&target);
        let mut out: Vec<&RuiRecord> = self
            .records
            .iter()
            .filter(|r| {
                if let Some(w) = &r.widget {
                    let w_low = w.to_ascii_lowercase();
                    w_low == target || singularize(&w_low) == singular
                } else {
                    false
                }
            })
            .collect();
        out.sort_by_key(|r| (kind_rank(&r.kind), r.line));
        out
    }

    /// Returns examples that use the specified widget (or all examples if `None`).
    pub fn examples_for_widget(&self, widget_name: Option<&str>) -> Vec<&RuiRecord> {
        let target = widget_name.map(|w| singularize(&w.trim().to_ascii_lowercase().replace('-', "_")));
        self.records
            .iter()
            .filter(|r| {
                if r.kind != "example" {
                    return false;
                }
                if let Some(t) = &target {
                    r.fields.iter().any(|f| singularize(&f.name) == *t || f.name.contains(t))
                        || r.name.to_ascii_lowercase().contains(t)
                } else {
                    true
                }
            })
            .collect()
    }

    /// The `kind` values actually present in this index, ordered for display in tool schemas.
    pub fn valid_kinds(&self) -> Vec<String> {
        let out: Vec<String> = self.meta.by_kind.keys().cloned().collect();
        out
    }

    /// Returns records closely related to `rec`: its owner's methods, siblings in the same module/widget,
    /// and records that reference it by name or field. Ordered by relation strength, then by `kind_rank`.
    pub fn related_records(&self, rec: &RuiRecord, limit: usize) -> Vec<&RuiRecord> {
        let name_key = norm_key(&rec.name);
        let scored: Vec<(usize, usize, &RuiRecord)> = self
            .records
            .iter()
            .map(|other| {
                if other.id == rec.id {
                    return (0usize, 0usize, other);
                }
                let rel = relation_score(rec, other, &name_key);
                (rel, kind_rank(&other.kind), other)
            })
            .filter(|t| t.0 > 0)
            .collect();
        let mut out: Vec<(usize, usize, &RuiRecord)> = scored;
        out.sort_by(|a, b| {
            b.0
                .cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.name.cmp(&b.2.name))
        });
        out.truncate(limit);
        let mut result: Vec<&RuiRecord> = Vec::new();
        for tuple in out {
            result.push(tuple.2);
        }
        result
    }

    /// Records whose documentation links to this record (reverse `linked_by` edges), ranked by how
    /// many distinct normalized keys of `rec` they link to, then by `kind_rank`. This is the only
    /// place the doc cross-reference graph is observable, so `rui_api` can state "referenced by"
    /// facts that trace back to actual `[...]` links in `bevy_rui` doc comments.
    pub fn referencing_records(&self, rec: &RuiRecord, limit: usize) -> Vec<&RuiRecord> {
        let mut keys: Vec<String> = vec![
            norm_key(&rec.name),
            norm_key(&rec.full_path),
        ];
        if rec.full_path.starts_with("bevy_rui::") {
            let stripped: Vec<String> = vec![
                rec.full_path.strip_prefix("bevy_rui::").unwrap_or("").to_string(),
                rec.full_path.strip_prefix("bevy_rui::widgets::").unwrap_or("").to_string(),
            ];
            keys.extend(stripped.iter().map(|s| norm_key(s)));
        }
        if let Some(owner) = &rec.owner {
            keys.push(norm_key(&format!("{owner}::{}", rec.name)));
        }
        keys.sort();
        keys.dedup();

        // Count, per record, how many of this record's alias keys some other record links to.
        let mut hits: HashMap<usize, usize> = HashMap::new();
        for k in &keys {
            if k.is_empty() {
                continue;
            }
            if let Some(indices) = self.linked_by.get(k) {
                for i in indices.iter() {
                    *hits.entry(*i).or_insert(0) += 1;
                }
            }
        }

        let mut scored: Vec<(usize, usize, usize, &RuiRecord)> = Vec::new();
        for (i, strength) in hits.iter() {
            let other = &self.records[*i];
            if other.id == rec.id {
                continue;
            }
            scored.push((*strength, kind_rank(&other.kind), *i, other));
        }
        scored.sort_by(|a, b| {
            b.0
                .cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        scored.truncate(limit);
        scored.iter().map(|t| t.3).collect()
    }

    /// Close indexed record names for a mistyped `symbol`, ranked by bounded Levenshtein distance.
    pub fn suggest_symbols(&self, symbol: &str, limit: usize) -> Vec<String> {
        let key = norm_key(symbol);
        if key.is_empty() {
            return Vec::new();
        }
        let key_len = key.chars().count();
        let max_dist = if key_len <= 3 { 1 } else if key_len <= 6 { 2 } else { 3 };
        let mut names: BTreeSet<String> = BTreeSet::new();
        for rec in &self.records {
            names.insert(rec.name.clone());
        }
        let mut matches: Vec<(usize, String)> = Vec::new();
        for name in &names {
            let cand = norm_key(name);
            if cand == key {
                continue;
            }
            let cand_len = cand.chars().count();
            if cand_len + max_dist < key_len || cand_len.saturating_sub(max_dist) > key_len {
                continue;
            }
            let dist = levenshtein_bounded(&cand, &key, max_dist);
            if dist > max_dist {
                continue;
            }
            matches.push((dist, name.clone()));
        }
        matches.sort_by(|a, b| {
            a.0
                .cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
        });
        matches.truncate(limit);
        matches.iter().map(|(_, name)| name.clone()).collect()
    }
}

fn kind_rank(kind: &str) -> usize {
    match kind {
        "module" => 0,
        "trait_method" => 1,
        "fn" => 2,
        "struct" => 3,
        "enum" => 4,
        "trait" => 5,
        "method" => 6,
        "example" => 7,
        _ => 8,
    }
}

/// Relation strength between two records: shared owner, shared widget, shared module, and name/field references.
fn relation_score(rec: &RuiRecord, other: &RuiRecord, name_key: &str) -> usize {
    let mut rel = 0usize;
    if let Some(owner) = &other.owner {
        if *owner == rec.name {
            rel += 6;
        }
    }
    if let Some(owner) = &rec.owner {
        if other.name == *owner {
            rel += 6;
        }
    }
    if let Some(w) = &rec.widget {
        if let Some(ow) = &other.widget {
            if ow == w {
                rel += 3;
            }
        }
    }
    if other.module == rec.module {
        rel += 2;
    }
    if name_key.len() >= 3 {
        if norm_key(&other.full_path).contains(name_key) {
            rel += 2;
        }
        if other.fields.iter().any(|f| norm_key(&f.name) == name_key) {
            rel += 2;
        }
    }
    if other.file == rec.file {
        rel += 1;
    }
    // Doc-link edges follow in both directions: a record that links to this one, or one this one
    // links to, is a genuine cross-reference rather than an accidental token overlap.
    for target in &rec.doc_links {
        if norm_key(target) == name_key || norm_key(target) == norm_key(&other.name) {
            rel += 4;
        }
    }
    for target in &other.doc_links {
        if norm_key(target) == name_key {
            rel += 4;
        }
    }
    rel
}

/// Levenshtein edit distance between `a` and `b`, short-circuiting to `max + 1` once the distance exceeds `max`.
pub fn levenshtein_bounded(a: &str, b: &str, max: usize) -> usize {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    if ac.is_empty() {
        return bc.len();
    }
    if bc.is_empty() {
        return ac.len();
    }
    let columns = bc.len() + 1;
    let mut prev: Vec<usize> = Vec::new();
    for i in 0..columns {
        prev.push(i);
    }
    let mut cur: Vec<usize> = Vec::new();
    for row in 1..=ac.len() {
        cur.clear();
        cur.push(row);
        let mut row_min = row;
        for col in 1..columns {
            let cost = if ac[row - 1] == bc[col - 1] { 0 } else { 1 };
            let deletion = prev[col] + 1;
            let insertion = cur[col - 1] + 1;
            let substitution = prev[col - 1] + cost;
            let mut best = deletion;
            if insertion < best {
                best = insertion;
            }
            if substitution < best {
                best = substitution;
            }
            cur.push(best);
            if best < row_min {
                row_min = best;
            }
        }
        if row_min > max {
            return max + 1;
        }
        for i in 0..columns {
            prev[i] = cur[i];
        }
    }
    prev[bc.len()]
}

/// Returns the version directory path inside `data_dir` (`<data_dir>/versions/<version>`).
pub fn version_dir(data_dir: &Path, version: &str) -> PathBuf {
    let safe: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '+' {
                c
            } else {
                '_'
            }
        })
        .collect();
    data_dir.join("versions").join(safe)
}

pub fn read_registry(data_dir: &Path) -> RegistryData {
    let path = data_dir.join("registry.json");
    if let Ok(text) = fs::read_to_string(path) {
        if let Ok(reg) = serde_json::from_str::<RegistryData>(&text) {
            return reg;
        }
    }
    RegistryData::default()
}

pub fn write_registry(data_dir: &Path, reg: &RegistryData) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join("registry.json");
    let json = serde_json::to_string_pretty(reg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Persists an index to `<data_dir>/versions/<version>/records.ndjson` and `meta.json`, updating `registry.json`.
pub fn save_index_to_disk(data_dir: &Path, index: &RuiIndex) -> Result<PathBuf, String> {
    let v_dir = version_dir(data_dir, &index.meta.rui_version);
    fs::create_dir_all(&v_dir).map_err(|e| format!("Failed to create {}: {e}", v_dir.display()))?;

    let records_path = v_dir.join("records.ndjson");
    {
        let file = File::create(&records_path).map_err(|e| e.to_string())?;
        let mut writer = BufWriter::new(file);
        for rec in &index.records {
            let line = serde_json::to_string(rec).map_err(|e| e.to_string())?;
            writer.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
    }

    let meta_path = v_dir.join("meta.json");
    // Persisted metadata is deliberately path-free: `rui_path` is a machine-local absolute path
    // (whose checkout happened to build this index), it means nothing on another machine, and the
    // persisted index is committed into this repo and repacked into release bundles, so keeping it
    // would bake one developer's project location into data every consumer downloads. Freshness comes
    // from `fingerprint` compared against `cfg.rui_path`, never from this field.
    let mut disk_meta = index.meta.clone();
    disk_meta.rui_path = None;
    let meta_json = serde_json::to_string_pretty(&disk_meta).map_err(|e| e.to_string())?;
    fs::write(&meta_path, meta_json).map_err(|e| e.to_string())?;

    let mut reg = read_registry(data_dir);
    reg.cache_version = CACHE_VERSION;
    reg.versions.insert(
        index.meta.rui_version.clone(),
        RegistryEntry {
            version: index.meta.rui_version.clone(),
            cache_version: CACHE_VERSION,
            built_at: index.meta.built_at.clone(),
            records: index.meta.total_records,
            symbols: index.meta.total_symbols,
            widgets: index.meta.widgets.clone(),
            // Same reason as `meta.json`: the registry is committed/shipped, so it never stores a
            // machine-local source path. The version, counts and fingerprint identify the index.
            rui_path: None,
            asset: None,
            revision: None,
            sha256: None,
        },
    );
    write_registry(data_dir, &reg)?;

    Ok(v_dir)
}

/// Loads a persisted index from `<data_dir>/versions/<version>/`.
pub fn load_index_from_disk(data_dir: &Path, version: &str) -> Result<RuiIndex, String> {
    let v_dir = version_dir(data_dir, version);
    let meta_path = v_dir.join("meta.json");
    let records_path = v_dir.join("records.ndjson");

    if !meta_path.is_file() || !records_path.is_file() {
        return Err(format!(
            "No persisted index found at {}",
            v_dir.display()
        ));
    }

    let meta_text = fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
    let mut meta: IndexMetadata = serde_json::from_str(&meta_text).map_err(|e| e.to_string())?;
    if meta.cache_version != CACHE_VERSION {
        if meta.cache_version == 0 || meta.cache_version > CACHE_VERSION {
            return Err(format!(
                "Persisted index cache version v{} is invalid or newer than current v{}",
                meta.cache_version,
                CACHE_VERSION,
            ))
        }
        // Older schemas are readable rather than rejected: each field added by a later schema
        // version carries a serde default, so an older `meta.json` + `records.ndjson` pair still
        // deserializes. Upgrade it in memory instead of failing, because a prebuilt index
        // downloaded from a GitHub Release may have been published by an older build of this MCP,
        // and refusing to load it would orphan every already-published bundle (which is exactly
        // what a cache-version bump did to v1 bundles before). The origin is recorded so `rui_status`
        // can report the schema the bundle really came from.
        meta.migrated_from_cache_version = Some(meta.cache_version)
    }
    meta.cache_version = CACHE_VERSION;

    let file = File::open(&records_path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let rec: RuiRecord = serde_json::from_str(trimmed).map_err(|e| e.to_string())?;
            records.push(rec);
        }
    }

    Ok(RuiIndex::from_records(records, meta))
}

/// Loads a valid cached index if up-to-date, or builds it directly from `cfg.rui_path` if available.
pub fn load_or_build_index(cfg: &ResolvedConfig, force: bool) -> Result<RuiIndex, String> {
    let current_fp = cfg
        .rui_path
        .as_deref()
        .map(compute_source_fingerprint);

    // 1. Try persisted cache if not forced
    if !force {
        if let Ok(cached) = load_index_from_disk(&cfg.data_dir, &cfg.rui_version) {
            // If we have a local checkout, verify its fingerprint matches so edits are picked up automatically
            if let Some(fp) = &current_fp {
                if cached.meta.fingerprint == *fp {
                    return Ok(cached);
                }
            } else {
                // No local checkout, use the prebuilt/fetched index from disk
                return Ok(cached);
            }
        }
    }

    // 2. Build from local `rui_path` source if available
    if let Some(rui_dir) = &cfg.rui_path {
        let fp = current_fp.unwrap_or_else(|| compute_source_fingerprint(rui_dir));
        let records = ingest_bevy_rui(rui_dir, &cfg.rui_version);
        let meta = IndexMetadata {
            cache_version: CACHE_VERSION,
            rui_version: cfg.rui_version.clone(),
            bevy_version: cfg.bevy_version.clone(),
            built_at: iso_timestamp(),
            fingerprint: fp,
            rui_path: Some(rui_dir.display().to_string()),
            total_records: records.len(),
            total_symbols: 0,
            widgets: Vec::new(),
            by_kind: BTreeMap::new(),
            total_doc_links: 0,
            total_implements: 0,
            migrated_from_cache_version: None,
        };
        let index = RuiIndex::from_records(records, meta);
        let _ = save_index_to_disk(&cfg.data_dir, &index);
        return Ok(index);
    }

    // 3. Fallback: check if any version is installed in registry
    let reg = read_registry(&cfg.data_dir);
    if let Some((ver, _)) = reg.versions.iter().next_back() {
        if let Ok(cached) = load_index_from_disk(&cfg.data_dir, ver) {
            return Ok(cached);
        }
    }

    Err(format!(
        "Neither a local `bevy_rui` directory nor a prebuilt index for v{} was found.\n\
         - Point `--rui-path <dir>` or `BEVY_RUI_PATH=<dir>` at your local `bevy_rui` checkout, OR\n\
         - Run `bevy-rui-mcp fetch-index {}` to download a prebuilt index from GitHub Releases.",
        cfg.rui_version, cfg.rui_version
    ))
}

/// Packages an installed index into a distributable `.tar.gz` bundle + `index-manifest.json` for GitHub Releases.
pub fn publish_index_bundle(
    cfg: &ResolvedConfig,
    out_dir: &Path,
    revision: u32,
) -> Result<(PathBuf, PathBuf), String> {
    let index = load_or_build_index(cfg, false)?;
    let v_dir = save_index_to_disk(&cfg.data_dir, &index)?;
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;

    let ver = &index.meta.rui_version;
    let bundle_name = if revision > 0 {
        format!("bevy-rui-index-{ver}+{revision}.tar.gz")
    } else {
        format!("bevy-rui-index-{ver}.tar.gz")
    };
    let bundle_path = out_dir.join(&bundle_name);
    // Normalized `meta.json` staged here only to be packed into the tar, then deleted.
    let dist_meta_path = out_dir.join(".bevy-rui-index-meta-dist.json");

    {
        let tar_gz = File::create(&bundle_path).map_err(|e| e.to_string())?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = tar::Builder::new(enc);

        // Ship a normalized copy of `meta.json` under the `versions/<ver>/meta.json` name: `rui_path`
        // is a machine-local absolute path (the checkout this index happened to be built from)
        // that means nothing on the consumer's machine and would bake one developer's project
        // location into distributed data, so a downloaded bundle carries no source path at all. A
        // migration marker inherited from an older republished index is dropped too, since this
        // publication is schema-current.
        {
            let mut dist_meta = index.meta.clone();
            dist_meta.rui_path = None;
            dist_meta.migrated_from_cache_version = None;
            let dist_json = serde_json::to_string_pretty(&dist_meta).map_err(|e| e.to_string())?;
            fs::write(&dist_meta_path, dist_json).map_err(|e| e.to_string())?;
        }
        tar.append_path_with_name(
            v_dir.join("records.ndjson"),
            format!("versions/{ver}/records.ndjson"),
        )
        .map_err(|e| e.to_string())?;
        tar.append_path_with_name(
            &dist_meta_path,
            format!("versions/{ver}/meta.json"),
        )
        .map_err(|e| e.to_string())?;
        tar.finish().map_err(|e| e.to_string())?;
    }

    // The normalized meta copy only exists to be packed into the tar; drop it from `dist/`.
    if dist_meta_path.is_file() {
        let _ = fs::remove_file(&dist_meta_path);
    }

    let bytes = fs::read(&bundle_path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = hex::encode(hasher.finalize());

    let manifest = ReleaseManifest {
        manifest_version: 1,
        rui_version: ver.clone(),
        asset: bundle_name,
        revision,
        sha256,
        cache_version: CACHE_VERSION,
        size: bytes.len() as u64,
        total_records: index.meta.total_records,
        built_at: index.meta.built_at.clone(),
    };

    let manifest_path = out_dir.join("index-manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, manifest_json).map_err(|e| e.to_string())?;

    Ok((bundle_path, manifest_path))
}

/// Downloads and unpacks a prebuilt `bevy-rui-index-<version>.tar.gz` from GitHub Releases or an explicit URL/file.
pub fn fetch_index_bundle(
    cfg: &ResolvedConfig,
    from_url_or_file: Option<&str>,
    force: bool,
) -> Result<PathBuf, String> {
    let ver = &cfg.rui_version;
    let target_v_dir = version_dir(&cfg.data_dir, ver);
    if !force && target_v_dir.join("records.ndjson").is_file() && target_v_dir.join("meta.json").is_file() {
        return Ok(target_v_dir);
    }

    fs::create_dir_all(&cfg.data_dir).map_err(|e| e.to_string())?;
    let tmp_archive = cfg.data_dir.join(format!(".fetch-{ver}.tar.gz"));

    let source = if let Some(explicit) = from_url_or_file {
        explicit.to_string()
    } else {
        format!(
            "https://github.com/{}/releases/download/v{ver}/bevy-rui-index-{ver}.tar.gz",
            cfg.repo
        )
    };

    if Path::new(&source).is_file() {
        fs::copy(&source, &tmp_archive).map_err(|e| format!("Failed to copy {source}: {e}"))?;
    } else {
        // Use curl for reliable HTTPS + redirect handling without heavy TLS crate dependencies
        let status = Command::new("curl")
            .args([
                "-fSL",
                "--retry",
                "2",
                "-H",
                "User-Agent: bevy-rui-mcp",
                "-o",
            ])
            .arg(&tmp_archive)
            .arg(&source)
            .status()
            .map_err(|e| format!("Failed to execute `curl` to download {source}: {e}"))?;

        if !status.success() {
            let _ = fs::remove_file(&tmp_archive);
            return Err(format!(
                "Failed to download prebuilt index from {source} (curl exited with {status})"
            ));
        }
    }

    // Extract archive safely into data_dir
    let extract_res = (|| -> Result<(), String> {
        let file = File::open(&tmp_archive).map_err(|e| e.to_string())?;
        let gz = GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            let path_str = path.to_string_lossy();
            if path.is_absolute() || path_str.contains("..") {
                return Err(format!("Unsafe archive entry path: {path_str}"));
            }
            entry.unpack_in(&cfg.data_dir).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();

    let _ = fs::remove_file(&tmp_archive);
    extract_res?;

    // Validate and register in registry.json
    let loaded = load_index_from_disk(&cfg.data_dir, ver)?;
    let mut reg = read_registry(&cfg.data_dir);
    reg.cache_version = CACHE_VERSION;
    reg.versions.insert(
        ver.clone(),
        RegistryEntry {
            version: ver.clone(),
            cache_version: CACHE_VERSION,
            built_at: loaded.meta.built_at.clone(),
            records: loaded.meta.total_records,
            symbols: loaded.meta.total_symbols,
            widgets: loaded.meta.widgets.clone(),
            rui_path: None,
            asset: Some(source),
            revision: Some(0),
            sha256: None,
        },
    );
    write_registry(&cfg.data_dir, &reg)?;

    // Persist the normalized current-schema copy (migration marker included) so later loads are
    // schema-clean and `rui_status` reports where an older bundle came from.
    if loaded.meta.migrated_from_cache_version.is_some() {
        let _ = save_index_to_disk(&cfg.data_dir, &loaded)?;
    }

    Ok(target_v_dir)
}
