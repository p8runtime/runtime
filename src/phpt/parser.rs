use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum PhptError {
    IoError(std::io::Error),
    MissingSection(String),
    InvalidFormat(String),
}

impl From<std::io::Error> for PhptError {
    fn from(err: std::io::Error) -> Self {
        PhptError::IoError(err)
    }
}

impl std::fmt::Display for PhptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PhptError::IoError(e) => write!(f, "IO error: {}", e),
            PhptError::MissingSection(s) => write!(f, "Missing required section: {}", s),
            PhptError::InvalidFormat(s) => write!(f, "Invalid format: {}", s),
        }
    }
}

impl std::error::Error for PhptError {}

#[derive(Debug, Clone)]
pub struct PhptTest {
    pub file_path: PathBuf,
    pub description: String,
    pub sections: PhptSections,
}

#[derive(Debug, Clone, Default)]
pub struct PhptSections {
    pub file: String,
    pub expect: Option<String>,
    pub expectf: Option<String>,
    pub expectregex: Option<String>,
    pub skipif: Option<String>,
    pub ini: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
    pub args: Option<String>,
    pub clean: Option<String>,
}

impl PhptTest {
    pub fn from_file(path: &Path) -> Result<Self, PhptError> {
        let content = fs::read_to_string(path)?;
        Self::from_string(&content, path.to_path_buf())
    }

    pub fn from_string(content: &str, file_path: PathBuf) -> Result<Self, PhptError> {
        let mut description = String::new();
        let mut sections = PhptSections::default();
        let mut current_section: Option<String> = None;
        let mut current_content = String::new();

        for line in content.lines() {
            // Check if this is a section marker
            if line.starts_with("--") && line.ends_with("--") && line.len() > 4 {
                // Save previous section
                if let Some(section_name) = current_section.take() {
                    Self::save_section(&section_name, &current_content, &mut sections, &mut description)?;
                    current_content.clear();
                }

                // Start new section
                let section_name = line[2..line.len()-2].to_string();
                current_section = Some(section_name);
            } else if current_section.is_some() {
                // Accumulate content for current section
                if !current_content.is_empty() {
                    current_content.push('\n');
                }
                current_content.push_str(line);
            }
        }

        // Save last section
        if let Some(section_name) = current_section {
            Self::save_section(&section_name, &current_content, &mut sections, &mut description)?;
        }

        // Validate required sections
        if description.is_empty() {
            return Err(PhptError::MissingSection("TEST".to_string()));
        }
        if sections.file.is_empty() {
            return Err(PhptError::MissingSection("FILE".to_string()));
        }
        if sections.expect.is_none() && sections.expectf.is_none() && sections.expectregex.is_none() {
            return Err(PhptError::MissingSection("EXPECT/EXPECTF/EXPECTREGEX".to_string()));
        }

        Ok(PhptTest {
            file_path,
            description: description.trim().to_string(),
            sections,
        })
    }

    fn save_section(
        name: &str,
        content: &str,
        sections: &mut PhptSections,
        description: &mut String,
    ) -> Result<(), PhptError> {
        let trimmed_content = content.trim_start();

        match name {
            "TEST" => {
                *description = trimmed_content.to_string();
            }
            "FILE" => {
                sections.file = trimmed_content.to_string();
            }
            "EXPECT" => {
                sections.expect = Some(trimmed_content.to_string());
            }
            "EXPECTF" => {
                sections.expectf = Some(trimmed_content.to_string());
            }
            "EXPECTREGEX" => {
                sections.expectregex = Some(trimmed_content.to_string());
            }
            "SKIPIF" => {
                sections.skipif = Some(trimmed_content.to_string());
            }
            "INI" => {
                for line in trimmed_content.lines() {
                    if let Some(pos) = line.find('=') {
                        let key = line[..pos].trim().to_string();
                        let value = line[pos + 1..].trim().to_string();
                        sections.ini.push((key, value));
                    }
                }
            }
            "ENV" => {
                for line in trimmed_content.lines() {
                    if let Some(pos) = line.find('=') {
                        let key = line[..pos].trim().to_string();
                        let value = line[pos + 1..].trim().to_string();
                        sections.env.push((key, value));
                    }
                }
            }
            "ARGS" => {
                sections.args = Some(trimmed_content.to_string());
            }
            "CLEAN" => {
                sections.clean = Some(trimmed_content.to_string());
            }
            "XLEAK" | "CREDITS" => {
                // Ignored sections
            }
            _ => {
                // Unknown section, ignore with warning
                eprintln!("Warning: Unknown section --{}--", name);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_phpt() {
        let content = r#"--TEST--
Simple echo test
--FILE--
<?php echo "Hello World"; ?>
--EXPECT--
Hello World
"#;
        let test = PhptTest::from_string(content, PathBuf::from("test.phpt")).unwrap();
        assert_eq!(test.description, "Simple echo test");
        assert_eq!(test.sections.file, "<?php echo \"Hello World\"; ?>");
        assert_eq!(test.sections.expect, Some("Hello World".to_string()));
    }

    #[test]
    fn test_parse_with_ini_and_env() {
        let content = r#"--TEST--
Test with settings
--INI--
error_reporting=E_ALL
display_errors=1
--ENV--
TEST_VAR=test_value
--FILE--
<?php echo getenv('TEST_VAR'); ?>
--EXPECT--
test_value
"#;
        let test = PhptTest::from_string(content, PathBuf::from("test.phpt")).unwrap();
        assert_eq!(test.sections.ini.len(), 2);
        assert_eq!(test.sections.ini[0], ("error_reporting".to_string(), "E_ALL".to_string()));
        assert_eq!(test.sections.env.len(), 1);
        assert_eq!(test.sections.env[0], ("TEST_VAR".to_string(), "test_value".to_string()));
    }
}
