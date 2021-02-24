use std::mem;
use tantivy::tokenizer::{BoxTokenStream, Token, TokenFilter, TokenStream};

impl TokenFilter for LetterReplacer {
    fn transform<'a>(&self, token_stream: BoxTokenStream<'a>) -> BoxTokenStream<'a> {
        BoxTokenStream::from(LetterReplacerTokenStream { tail: token_stream })
    }
}

#[derive(Clone)]
pub struct LetterReplacer;

pub struct LetterReplacerTokenStream<'a> {
    tail: BoxTokenStream<'a>,
}

impl<'a> TokenStream for LetterReplacerTokenStream<'a> {
    fn advance(&mut self) -> bool {
        if !self.tail.advance() {
            return false;
        }
        // replace lowercase cyrillic 'YO' (ё) to 'E' (е)
        //TODO: pass replacement map as filter parameter
        if self.tail.token_mut().text.contains('\u{0451}') {
            let mut r = self.tail.token_mut().text.replace('\u{0451}', "\u{0435}");
            mem::swap(&mut self.tail.token_mut().text, &mut r);
        }
        true
    }

    fn token(&self) -> &Token {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut Token {
        self.tail.token_mut()
    }
}

#[cfg(test)]
mod tests {
    use tantivy::tokenizer::{SimpleTokenizer, TextAnalyzer};

    #[test]
    fn test_replace() {
        assert_eq!(
            replace_helper("test: ах у ели, ах у ёлки"),
            vec![
                "test".to_string(),
                "ах".to_string(),
                "у".to_string(),
                "ели".to_string(),
                "ах".to_string(),
                "у".to_string(),
                "елки".to_string()
            ]
        );
    }

    fn replace_helper(text: &str) -> Vec<String> {
        let mut tokens = vec![];
        let mut token_stream = TextAnalyzer::from(SimpleTokenizer)
            .filter(crate::letter_replacer::LetterReplacer)
            .token_stream(text);
        while token_stream.advance() {
            let token_text = token_stream.token().text.clone();
            tokens.push(token_text);
        }
        tokens
    }
}
