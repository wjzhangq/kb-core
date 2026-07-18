use tantivy::schema::{
    Field, Schema, TextFieldIndexing, TextOptions,
    FAST, STORED,
};

pub struct TantivySchema {
    pub schema: Schema,
    pub chunk_id: Field,
    pub doc_id: Field,
    pub doc_type: Field,
    pub text: Field,
    pub title: Field,
    pub path: Field,
}

impl TantivySchema {
    pub fn build() -> Self {
        let mut builder = Schema::builder();

        // Numeric stored+fast fields for retrieval
        let chunk_id = builder.add_u64_field("chunk_id", FAST | STORED);
        let doc_id = builder.add_u64_field("doc_id", FAST | STORED);

        // Stored-only fields
        let doc_type = builder.add_text_field(
            "doc_type",
            TextOptions::default().set_stored(),
        );
        let path = builder.add_text_field(
            "path",
            TextOptions::default().set_stored(),
        );

        // Full-text indexed + stored with jieba+lowercase tokenizer
        let text_indexing = TextFieldIndexing::default()
            .set_tokenizer("jieba_lower")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions);
        let text_options = TextOptions::default()
            .set_indexing_options(text_indexing)
            .set_stored();
        let text = builder.add_text_field("text", text_options.clone());

        let title_indexing = TextFieldIndexing::default()
            .set_tokenizer("jieba_lower")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions);
        let title_options = TextOptions::default()
            .set_indexing_options(title_indexing)
            .set_stored();
        let title = builder.add_text_field("title", title_options);

        TantivySchema {
            schema: builder.build(),
            chunk_id,
            doc_id,
            doc_type,
            text,
            title,
            path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_all_fields() {
        let ts = TantivySchema::build();
        assert!(ts.schema.get_field("chunk_id").is_ok());
        assert!(ts.schema.get_field("doc_id").is_ok());
        assert!(ts.schema.get_field("doc_type").is_ok());
        assert!(ts.schema.get_field("text").is_ok());
        assert!(ts.schema.get_field("title").is_ok());
        assert!(ts.schema.get_field("path").is_ok());
    }
}
