//! Custom search (rule builder + quick query, CLN-ENG-*). Owner: W3-D.

use mj_core::{Error, Opportunity, Result};

use crate::api::{SearchParseRequest, SearchParseResponse, SearchRunRequest, SearchSuggestRequest, Suggestion};
use crate::Engine;

impl Engine {
    /// Parse a quick query.
    pub fn search_parse(&self, req: SearchParseRequest) -> Result<SearchParseResponse> {
        Ok(match mj_filters::parse(&req.query) {
            Ok(rule) => SearchParseResponse { rule: Some(rule), errors: Vec::new() },
            Err(errors) => SearchParseResponse { rule: None, errors },
        })
    }

    /// Autocomplete.
    pub fn search_suggest(&self, req: SearchSuggestRequest) -> Result<Vec<Suggestion>> {
        Ok(mj_filters::suggest(&req.query, req.cursor))
    }

    /// Run a rule over the index as a custom opportunity.
    pub fn search_run(&self, req: SearchRunRequest) -> Result<Opportunity> {
        let _ = req;
        Err(Error::not_implemented("custom search (W3-D)"))
    }
}
