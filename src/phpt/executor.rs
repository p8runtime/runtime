use crate::compiler::emitter::Emitter;
use crate::parser::lexer::Lexer;
use crate::parser::parser::Parser as PhpParser;
use crate::phpt::matcher::{match_output, ExpectationType};
use crate::phpt::output_writer::BufferedOutputWriter;
use crate::phpt::parser::PhptTest;
use crate::runtime::context::{EngineBuilder, EngineContext};
use crate::vm::engine::VM;
use bumpalo::Bump;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum TestResult {
    Passed,
    Failed { expected: String, actual: String },
    Skipped { reason: String },
    Error { error: String },
}

pub struct PhptExecutor {
    engine_context: Arc<EngineContext>,
}

impl PhptExecutor {
    pub fn new() -> Result<Self, String> {
        let builder = EngineBuilder::new();
        let engine_context = builder
            .with_core_extensions()
            .build()
            .map_err(|e| format!("Failed to build engine: {}", e))?;

        Ok(Self { engine_context })
    }

    pub fn run_test(&self, test: &PhptTest) -> TestResult {
        // Check SKIPIF first
        if let Some(ref skipif_code) = test.sections.skipif {
            match self.execute_skipif(skipif_code) {
                Ok(Some(reason)) => return TestResult::Skipped { reason },
                Err(e) => return TestResult::Error {
                    error: format!("SKIPIF error: {}", e),
                },
                Ok(None) => {
                    // Continue with test
                }
            }
        }

        // Apply environment variables
        for (key, value) in &test.sections.env {
            unsafe {
                std::env::set_var(key, value);
            }
        }

        // Execute the test
        let result = self.execute_test_code(test);

        // Clean up environment variables
        for (key, _) in &test.sections.env {
            unsafe {
                std::env::remove_var(key);
            }
        }

        // Run CLEAN section if present
        if let Some(ref clean_code) = test.sections.clean {
            if let Err(e) = self.execute_clean(clean_code) {
                eprintln!("Warning: CLEAN section failed: {}", e);
            }
        }

        result
    }

    fn execute_skipif(&self, skipif_code: &str) -> Result<Option<String>, String> {
        let mut vm = VM::new_with_sapi(self.engine_context.clone(), crate::sapi::SapiMode::Cli);
        let output_writer = BufferedOutputWriter::new();
        vm.set_output_writer(Box::new(output_writer.clone()));

        if let Err(e) = self.execute_source(skipif_code, &mut vm) {
            return Err(format!("Failed to execute SKIPIF: {}", e));
        }

        let output = output_writer.get_output();

        // Check if output contains "skip"
        if output.to_lowercase().contains("skip") {
            // Extract reason (everything after "skip")
            let reason = output
                .lines()
                .find(|line| line.to_lowercase().contains("skip"))
                .and_then(|line| {
                    let lower = line.to_lowercase();
                    lower.find("skip").map(|pos| {
                        let after = &line[pos + 4..];
                        after.trim().to_string()
                    })
                })
                .unwrap_or_else(|| "Test skipped".to_string());

            Ok(Some(reason))
        } else {
            Ok(None)
        }
    }

    fn execute_test_code(&self, test: &PhptTest) -> TestResult {
        let mut vm = VM::new_with_sapi(self.engine_context.clone(), crate::sapi::SapiMode::Cli);

        // TODO: Apply INI settings (when INI handling is available)
        // For now, we'll skip INI settings

        // TODO: Set up $argv and $argc from ARGS section
        // For now, we'll skip command-line arguments

        let output_writer = BufferedOutputWriter::new();
        vm.set_output_writer(Box::new(output_writer.clone()));

        if let Err(e) = self.execute_source(&test.sections.file, &mut vm) {
            return TestResult::Error {
                error: format!("Execution error: {}", e),
            };
        }

        let actual_output = output_writer.get_output();

        // Determine expected output type
        let expected = if let Some(ref expect) = test.sections.expect {
            ExpectationType::Exact(expect.clone())
        } else if let Some(ref expectf) = test.sections.expectf {
            ExpectationType::Format(expectf.clone())
        } else if let Some(ref expectregex) = test.sections.expectregex {
            ExpectationType::Regex(expectregex.clone())
        } else {
            return TestResult::Error {
                error: "No EXPECT/EXPECTF/EXPECTREGEX section found".to_string(),
            };
        };

        if match_output(&actual_output, expected.clone()) {
            TestResult::Passed
        } else {
            let expected_str = match expected {
                ExpectationType::Exact(s) => s,
                ExpectationType::Format(s) => format!("EXPECTF: {}", s),
                ExpectationType::Regex(s) => format!("EXPECTREGEX: {}", s),
            };

            TestResult::Failed {
                expected: expected_str,
                actual: actual_output,
            }
        }
    }

    fn execute_clean(&self, clean_code: &str) -> Result<(), String> {
        let mut vm = VM::new_with_sapi(self.engine_context.clone(), crate::sapi::SapiMode::Cli);
        let output_writer = BufferedOutputWriter::new();
        vm.set_output_writer(Box::new(output_writer));

        self.execute_source(clean_code, &mut vm)
            .map_err(|e| format!("CLEAN failed: {}", e))
    }

    fn execute_source(&self, source: &str, vm: &mut VM) -> Result<(), String> {
        let source_bytes = source.as_bytes();
        let arena = Bump::new();
        let lexer = Lexer::new(source_bytes);
        let mut parser = PhpParser::new(lexer, &arena);

        let program = parser.parse_program();

        if !program.errors.is_empty() {
            let mut error_msg = String::new();
            for error in program.errors {
                error_msg.push_str(&error.to_human_readable(source_bytes));
                error_msg.push('\n');
            }
            return Err(error_msg);
        }

        // Compile
        let emitter = Emitter::new(source_bytes, &mut vm.context.interner);
        let (chunk, has_error) = emitter.compile(program.statements);

        if has_error {
            return Err("Compilation errors occurred".to_string());
        }

        // Execute
        if let Err(e) = vm.run(Rc::new(chunk)) {
            return Err(format!("Runtime error: {:?}", e));
        }

        // Flush output buffers
        crate::builtins::output_control::flush_all_output_buffers(vm)
            .map_err(|e| format!("Failed to flush output: {:?}", e))?;
        vm.flush_output()
            .map_err(|e| format!("Failed to flush output: {:?}", e))?;

        Ok(())
    }
}

impl Default for PhptExecutor {
    fn default() -> Self {
        Self::new().expect("Failed to create PhptExecutor")
    }
}
