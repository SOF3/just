use crate::common::*;

pub(crate) struct AssignmentEvaluator<'a: 'b, 'b> {
  pub(crate) assignments: &'b BTreeMap<&'a str, Expression<'a>>,
  pub(crate) invocation_directory: &'b Result<PathBuf, String>,
  pub(crate) dotenv: &'b BTreeMap<String, String>,
  pub(crate) dry_run: bool,
  pub(crate) evaluated: BTreeMap<&'a str, String>,
  pub(crate) exports: &'b BTreeSet<&'a str>,
  pub(crate) overrides: &'b BTreeMap<&'b str, &'b str>,
  pub(crate) quiet: bool,
  pub(crate) scope: &'b BTreeMap<&'a str, String>,
  pub(crate) shell: &'b str,
}

impl<'a, 'b> AssignmentEvaluator<'a, 'b> {
  pub(crate) fn evaluate_assignments(
    assignments: &BTreeMap<&'a str, Expression<'a>>,
    invocation_directory: &Result<PathBuf, String>,
    dotenv: &'b BTreeMap<String, String>,
    overrides: &BTreeMap<&str, &str>,
    quiet: bool,
    shell: &'a str,
    dry_run: bool,
  ) -> RunResult<'a, BTreeMap<&'a str, String>> {
    let mut evaluator = AssignmentEvaluator {
      evaluated: empty(),
      exports: &empty(),
      scope: &empty(),
      assignments,
      invocation_directory,
      dotenv,
      dry_run,
      overrides,
      quiet,
      shell,
    };

    for name in assignments.keys() {
      evaluator.evaluate_assignment(name)?;
    }

    Ok(evaluator.evaluated)
  }

  pub(crate) fn evaluate_line(
    &mut self,
    line: &[Fragment<'a>],
    arguments: &BTreeMap<&str, Cow<str>>,
  ) -> RunResult<'a, String> {
    let mut evaluated = String::new();
    for fragment in line {
      match *fragment {
        Fragment::Text { ref text } => evaluated += text.lexeme(),
        Fragment::Expression { ref expression } => {
          evaluated += &self.evaluate_expression(expression, arguments)?;
        }
      }
    }
    Ok(evaluated)
  }

  fn evaluate_assignment(&mut self, name: &'a str) -> RunResult<'a, ()> {
    if self.evaluated.contains_key(name) {
      return Ok(());
    }

    if let Some(expression) = self.assignments.get(name) {
      if let Some(value) = self.overrides.get(name) {
        self.evaluated.insert(name, value.to_string());
      } else {
        let value = self.evaluate_expression(expression, &empty())?;
        self.evaluated.insert(name, value);
      }
    } else {
      return Err(RuntimeError::Internal {
        message: format!("attempted to evaluated unknown assignment {}", name),
      });
    }

    Ok(())
  }

  pub(crate) fn evaluate_expression(
    &mut self,
    expression: &Expression<'a>,
    arguments: &BTreeMap<&str, Cow<str>>,
  ) -> RunResult<'a, String> {
    match *expression {
      Expression::Variable { name, .. } => {
        if self.evaluated.contains_key(name) {
          Ok(self.evaluated[name].clone())
        } else if self.scope.contains_key(name) {
          Ok(self.scope[name].clone())
        } else if self.assignments.contains_key(name) {
          self.evaluate_assignment(name)?;
          Ok(self.evaluated[name].clone())
        } else if arguments.contains_key(name) {
          Ok(arguments[name].to_string())
        } else {
          Err(RuntimeError::Internal {
            message: format!("attempted to evaluate undefined variable `{}`", name),
          })
        }
      }
      Expression::Call {
        name,
        arguments: ref call_arguments,
        ref token,
      } => {
        let call_arguments = call_arguments
          .iter()
          .map(|argument| self.evaluate_expression(argument, arguments))
          .collect::<Result<Vec<String>, RuntimeError>>()?;
        let context = FunctionContext {
          invocation_directory: &self.invocation_directory,
          dotenv: self.dotenv,
        };
        Function::evaluate(token, name, &context, &call_arguments)
      }
      Expression::String { ref cooked_string } => Ok(cooked_string.cooked.to_string()),
      Expression::Backtick { raw, ref token } => {
        if self.dry_run {
          Ok(format!("`{}`", raw))
        } else {
          Ok(self.run_backtick(self.dotenv, raw, token)?)
        }
      }
      Expression::Concatination { ref lhs, ref rhs } => {
        Ok(self.evaluate_expression(lhs, arguments)? + &self.evaluate_expression(rhs, arguments)?)
      }
      Expression::Group { ref expression } => self.evaluate_expression(&expression, arguments),
    }
  }

  fn run_backtick(
    &self,
    dotenv: &BTreeMap<String, String>,
    raw: &str,
    token: &Token<'a>,
  ) -> RunResult<'a, String> {
    let mut cmd = Command::new(self.shell);

    cmd.arg("-cu").arg(raw);

    cmd.export_environment_variables(self.scope, dotenv, self.exports)?;

    cmd.stdin(process::Stdio::inherit());

    cmd.stderr(if self.quiet {
      process::Stdio::null()
    } else {
      process::Stdio::inherit()
    });

    InterruptHandler::guard(|| {
      output(cmd).map_err(|output_error| RuntimeError::Backtick {
        token: token.clone(),
        output_error,
      })
    })
  }
}

#[cfg(test)]
mod test {
  use super::*;
  use crate::testing::parse;

  #[test]
  fn backtick_code() {
    match parse("a:\n echo {{`f() { return 100; }; f`}}")
      .run(&["a"], &Default::default())
      .unwrap_err()
    {
      RuntimeError::Backtick {
        token,
        output_error: OutputError::Code(code),
      } => {
        assert_eq!(code, 100);
        assert_eq!(token.lexeme(), "`f() { return 100; }; f`");
      }
      other => panic!("expected a code run error, but got: {}", other),
    }
  }

  #[test]
  fn export_assignment_backtick() {
    let text = r#"
export exported_variable = "A"
b = `echo $exported_variable`

recipe:
  echo {{b}}
"#;
    let config = Config {
      quiet: true,
      ..Default::default()
    };

    match parse(text).run(&["recipe"], &config).unwrap_err() {
      RuntimeError::Backtick {
        token,
        output_error: OutputError::Code(_),
      } => {
        assert_eq!(token.lexeme(), "`echo $exported_variable`");
      }
      other => panic!("expected a backtick code errror, but got: {}", other),
    }
  }
}
