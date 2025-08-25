use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct Headers {
    headers: HashMap<String, String>,
}

impl Headers {
    #[must_use]
    pub fn new() -> Headers {
        Headers {
            headers: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Headers {
        Headers {
            headers: HashMap::with_capacity(capacity),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_string(), value.to_string());
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&String> {
        self.headers.get(key)
    }

    /// Returns all the header entries. Note that there is no ordering in here!
    #[must_use]
    pub fn all(&self) -> &HashMap<String, String> {
        &self.headers
    }

    #[must_use]
    pub fn sorted(&self) -> Vec<(&String, &String)> {
        let mut sorted = self.headers.iter().collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headers() {
        let mut headers = Headers::new();

        headers.set("Content-Type", "application/json");
        assert_eq!(headers.get("Content-Type").unwrap(), "application/json");

        headers.set("Content-Type", "text/html");
        assert_eq!(headers.get("Content-Type").unwrap(), "text/html");
        assert_eq!(headers.all().len(), 1);
    }
}
