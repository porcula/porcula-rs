use std::mem;
use tantivy::tokenizer::{Token, TokenFilter, TokenStream, Tokenizer};

#[derive(Clone)]
pub struct LetterReplacer;

impl TokenFilter for LetterReplacer {
    type Tokenizer<T: Tokenizer> = LetterReplacerFilter<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> Self::Tokenizer<T> {
        LetterReplacerFilter {
            tokenizer,
            buffer: String::new(),
        }
    }
}

#[derive(Clone)]
pub struct LetterReplacerFilter<T> {
    tokenizer: T,
    buffer: String,
}

impl<T: Tokenizer> Tokenizer for LetterReplacerFilter<T> {
    type TokenStream<'a> = LetterReplacerTokenStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        self.buffer.clear();
        LetterReplacerTokenStream {
            tail: self.tokenizer.token_stream(text),
        }
    }
}

pub struct LetterReplacerTokenStream<T> {
    tail: T,
}

impl<T: TokenStream> TokenStream for LetterReplacerTokenStream<T> {
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
        let mut tokenizer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(crate::letter_replacer::LetterReplacer)
            .build();
        let mut token_stream = tokenizer.token_stream(text);
        while token_stream.advance() {
            let token_text = token_stream.token().text.clone();
            tokens.push(token_text);
        }
        tokens
    }
}
