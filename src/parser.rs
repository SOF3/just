use crate::common::*;

use CompilationErrorKind::*;
use TokenKind::*;

pub(crate) struct Parser<'a> {
  text: &'a str,
  tokens: itertools::PutBackN<vec::IntoIter<Token<'a>>>,
  recipes: BTreeMap<&'a str, Recipe<'a>>,
  assignments: BTreeMap<&'a str, Expression<'a>>,
  assignment_tokens: BTreeMap<&'a str, Token<'a>>,
  exports: BTreeSet<&'a str>,
  aliases: BTreeMap<&'a str, Alias<'a>>,
  alias_tokens: BTreeMap<&'a str, Token<'a>>,
  warnings: Vec<Warning<'a>>,
}

impl<'a> Parser<'a> {
  pub(crate) fn parse(text: &'a str) -> CompilationResult<'a, Justfile> {
    let mut tokens = Lexer::lex(text)?;
    tokens.retain(|token| token.kind != Whitespace);
    let parser = Parser::new(text, tokens);
    parser.justfile()
  }

  pub(crate) fn new(text: &'a str, tokens: Vec<Token<'a>>) -> Parser<'a> {
    Parser {
      tokens: itertools::put_back_n(tokens),
      recipes: empty(),
      assignments: empty(),
      assignment_tokens: empty(),
      exports: empty(),
      aliases: empty(),
      alias_tokens: empty(),
      warnings: Vec::new(),
      text,
    }
  }

  fn peek(&mut self, kind: TokenKind) -> bool {
    let next = self.tokens.next().unwrap();
    let result = next.kind == kind;
    self.tokens.put_back(next);
    result
  }

  fn accept(&mut self, kind: TokenKind) -> Option<Token<'a>> {
    if self.peek(kind) {
      self.tokens.next()
    } else {
      None
    }
  }

  fn accepted(&mut self, kind: TokenKind) -> bool {
    self.accept(kind).is_some()
  }

  fn expect(&mut self, kind: TokenKind) -> Option<Token<'a>> {
    if self.peek(kind) {
      self.tokens.next();
      None
    } else {
      self.tokens.next()
    }
  }

  fn expect_eol(&mut self) -> Option<Token<'a>> {
    self.accepted(Comment);
    if self.peek(Eol) {
      self.accept(Eol);
      None
    } else if self.peek(Eof) {
      None
    } else {
      self.tokens.next()
    }
  }

  fn unexpected_token(&self, found: &Token<'a>, expected: &[TokenKind]) -> CompilationError<'a> {
    found.error(UnexpectedToken {
      expected: expected.to_vec(),
      found: found.kind,
    })
  }

  fn recipe(
    &mut self,
    name: &Token<'a>,
    doc: Option<Token<'a>>,
    quiet: bool,
  ) -> CompilationResult<'a, ()> {
    if let Some(recipe) = self.recipes.get(name.lexeme()) {
      return Err(name.error(DuplicateRecipe {
        recipe: recipe.name,
        first: recipe.line_number,
      }));
    }

    let mut parsed_parameter_with_default = false;
    let mut parsed_variadic_parameter = false;
    let mut parameters: Vec<Parameter> = vec![];
    loop {
      let plus = self.accept(Plus);

      let parameter = match self.accept(Name) {
        Some(parameter) => parameter,
        None => {
          if let Some(plus) = plus {
            return Err(self.unexpected_token(&plus, &[Name]));
          } else {
            break;
          }
        }
      };

      let variadic = plus.is_some();

      if parsed_variadic_parameter {
        return Err(parameter.error(ParameterFollowsVariadicParameter {
          parameter: parameter.lexeme(),
        }));
      }

      if parameters.iter().any(|p| p.name == parameter.lexeme()) {
        return Err(parameter.error(DuplicateParameter {
          recipe: name.lexeme(),
          parameter: parameter.lexeme(),
        }));
      }

      let default;
      if self.accepted(Equals) {
        default = Some(self.value()?);
      } else {
        default = None
      }

      if parsed_parameter_with_default && default.is_none() {
        return Err(parameter.error(RequiredParameterFollowsDefaultParameter {
          parameter: parameter.lexeme(),
        }));
      }

      parsed_parameter_with_default |= default.is_some();
      parsed_variadic_parameter = variadic;

      parameters.push(Parameter {
        name: parameter.lexeme(),
        token: parameter,
        default,
        variadic,
      });
    }

    if let Some(token) = self.expect(Colon) {
      // if we haven't accepted any parameters, a :=
      // would have been fine as part of an assignment
      if parameters.is_empty() {
        return Err(self.unexpected_token(&token, &[Name, Plus, Colon, ColonEquals]));
      } else {
        return Err(self.unexpected_token(&token, &[Name, Plus, Colon]));
      }
    }

    let mut dependencies = vec![];
    let mut dependency_tokens = vec![];
    while let Some(dependency) = self.accept(Name) {
      if dependencies.contains(&dependency.lexeme()) {
        return Err(dependency.error(DuplicateDependency {
          recipe: name.lexeme(),
          dependency: dependency.lexeme(),
        }));
      }
      dependencies.push(dependency.lexeme());
      dependency_tokens.push(dependency);
    }

    if let Some(token) = self.expect_eol() {
      return Err(self.unexpected_token(&token, &[Name, Eol, Eof]));
    }

    let mut lines: Vec<Vec<Fragment>> = vec![];
    let mut shebang = false;

    if self.accepted(Indent) {
      while !self.accepted(Dedent) {
        if self.accepted(Eol) {
          lines.push(vec![]);
          continue;
        }
        if let Some(token) = self.expect(Line) {
          return Err(token.error(Internal {
            message: format!("Expected a line but got {}", token.kind),
          }));
        }
        let mut fragments = vec![];

        while !(self.accepted(Eol) || self.peek(Dedent)) {
          if let Some(token) = self.accept(Text) {
            if fragments.is_empty() {
              if lines.is_empty() {
                if token.lexeme().starts_with("#!") {
                  shebang = true;
                }
              } else if !shebang
                && !lines
                  .last()
                  .and_then(|line| line.last())
                  .map(Fragment::continuation)
                  .unwrap_or(false)
                && (token.lexeme().starts_with(' ') || token.lexeme().starts_with('\t'))
              {
                return Err(token.error(ExtraLeadingWhitespace));
              }
            }
            fragments.push(Fragment::Text { text: token });
          } else if let Some(token) = self.expect(InterpolationStart) {
            return Err(self.unexpected_token(&token, &[Text, InterpolationStart, Eol]));
          } else {
            fragments.push(Fragment::Expression {
              expression: self.expression()?,
            });

            if let Some(token) = self.expect(InterpolationEnd) {
              return Err(self.unexpected_token(&token, &[Plus, InterpolationEnd]));
            }
          }
        }

        lines.push(fragments);
      }
    }

    while lines.last().map(Vec::is_empty).unwrap_or(false) {
      lines.pop();
    }

    self.recipes.insert(
      name.lexeme(),
      Recipe {
        line_number: name.line,
        name: name.lexeme(),
        doc: doc.map(|t| t.lexeme()[1..].trim()),
        private: &name.lexeme()[0..1] == "_",
        dependencies,
        dependency_tokens,
        lines,
        parameters,
        quiet,
        shebang,
      },
    );

    Ok(())
  }

  fn value(&mut self) -> CompilationResult<'a, Expression<'a>> {
    let first = self.tokens.next().unwrap();

    match first.kind {
      Name => {
        if self.peek(ParenL) {
          if let Some(token) = self.expect(ParenL) {
            return Err(self.unexpected_token(&token, &[ParenL]));
          }
          let arguments = self.arguments()?;
          if let Some(token) = self.expect(ParenR) {
            return Err(self.unexpected_token(&token, &[Name, StringCooked, ParenR]));
          }
          Ok(Expression::Call {
            name: first.lexeme(),
            token: first,
            arguments,
          })
        } else {
          Ok(Expression::Variable {
            name: first.lexeme(),
            token: first,
          })
        }
      }
      Backtick => Ok(Expression::Backtick {
        raw: &first.lexeme()[1..first.lexeme().len() - 1],
        token: first,
      }),
      StringRaw | StringCooked => Ok(Expression::String {
        cooked_string: StringLiteral::new(&first)?,
      }),
      ParenL => {
        let expression = self.expression()?;

        if let Some(token) = self.expect(ParenR) {
          return Err(self.unexpected_token(&token, &[ParenR]));
        }

        Ok(Expression::Group {
          expression: Box::new(expression),
        })
      }
      _ => Err(self.unexpected_token(&first, &[Name, StringCooked])),
    }
  }

  fn expression(&mut self) -> CompilationResult<'a, Expression<'a>> {
    let lhs = self.value()?;

    if self.accepted(Plus) {
      let rhs = self.expression()?;

      Ok(Expression::Concatination {
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
      })
    } else {
      Ok(lhs)
    }
  }

  fn arguments(&mut self) -> CompilationResult<'a, Vec<Expression<'a>>> {
    let mut arguments = Vec::new();

    while !self.peek(ParenR) && !self.peek(Eof) && !self.peek(Eol) && !self.peek(InterpolationEnd) {
      arguments.push(self.expression()?);
      if !self.accepted(Comma) {
        if self.peek(ParenR) {
          break;
        } else {
          let next = self.tokens.next().unwrap();
          return Err(self.unexpected_token(&next, &[Comma, ParenR]));
        }
      }
    }

    Ok(arguments)
  }

  fn assignment(&mut self, name: Token<'a>, export: bool) -> CompilationResult<'a, ()> {
    if self.assignments.contains_key(name.lexeme()) {
      return Err(name.error(DuplicateVariable {
        variable: name.lexeme(),
      }));
    }
    if export {
      self.exports.insert(name.lexeme());
    }

    let expression = self.expression()?;
    if let Some(token) = self.expect_eol() {
      return Err(self.unexpected_token(&token, &[Plus, Eol]));
    }

    self.assignments.insert(name.lexeme(), expression);
    self.assignment_tokens.insert(name.lexeme(), name);
    Ok(())
  }

  fn alias(&mut self, name: Token<'a>) -> CompilationResult<'a, ()> {
    // Make sure alias doesn't already exist
    if let Some(alias) = self.aliases.get(name.lexeme()) {
      return Err(name.error(DuplicateAlias {
        alias: alias.name,
        first: alias.line_number,
      }));
    }

    // Make sure the next token is of kind Name and keep it
    let target = if let Some(next) = self.accept(Name) {
      next.lexeme()
    } else {
      let unexpected = self.tokens.next().unwrap();
      return Err(self.unexpected_token(&unexpected, &[Name]));
    };

    // Make sure this is where the line or file ends without any unexpected tokens.
    if let Some(token) = self.expect_eol() {
      return Err(self.unexpected_token(&token, &[Eol, Eof]));
    }

    self.aliases.insert(
      name.lexeme(),
      Alias {
        name: name.lexeme(),
        line_number: name.line,
        private: name.lexeme().starts_with('_'),
        target,
      },
    );
    self.alias_tokens.insert(name.lexeme(), name);

    Ok(())
  }

  pub(crate) fn justfile(mut self) -> CompilationResult<'a, Justfile<'a>> {
    let mut doc = None;
    loop {
      match self.tokens.next() {
        Some(token) => match token.kind {
          Eof => break,
          Eol => {
            doc = None;
            continue;
          }
          Comment => {
            if let Some(token) = self.expect_eol() {
              return Err(token.error(Internal {
                message: format!("found comment followed by {}", token.kind),
              }));
            }
            doc = Some(token);
          }
          At => {
            if let Some(name) = self.accept(Name) {
              self.recipe(&name, doc, true)?;
              doc = None;
            } else {
              let unexpected = &self.tokens.next().unwrap();
              return Err(self.unexpected_token(unexpected, &[Name]));
            }
          }
          Name => {
            if token.lexeme() == "export" {
              let next = self.tokens.next().unwrap();
              if next.kind == Name && self.peek(Equals) {
                self.warnings.push(Warning::DeprecatedEquals {
                  equals: self.tokens.next().unwrap(),
                });
                self.assignment(next, true)?;
                doc = None;
              } else if next.kind == Name && self.accepted(ColonEquals) {
                self.assignment(next, true)?;
                doc = None;
              } else {
                self.tokens.put_back(next);
                self.recipe(&token, doc, false)?;
                doc = None;
              }
            } else if token.lexeme() == "alias" {
              let next = self.tokens.next().unwrap();
              if next.kind == Name && self.peek(Equals) {
                self.warnings.push(Warning::DeprecatedEquals {
                  equals: self.tokens.next().unwrap(),
                });
                self.alias(next)?;
                doc = None;
              } else if next.kind == Name && self.accepted(ColonEquals) {
                self.alias(next)?;
                doc = None;
              } else {
                self.tokens.put_back(next);
                self.recipe(&token, doc, false)?;
                doc = None;
              }
            } else if self.peek(Equals) {
              self.warnings.push(Warning::DeprecatedEquals {
                equals: self.tokens.next().unwrap(),
              });
              self.assignment(token, false)?;
              doc = None;
            } else if self.accepted(ColonEquals) {
              self.assignment(token, false)?;
              doc = None;
            } else {
              self.recipe(&token, doc, false)?;
              doc = None;
            }
          }
          _ => return Err(self.unexpected_token(&token, &[Name, At])),
        },
        None => {
          return Err(CompilationError {
            text: self.text,
            offset: 0,
            line: 0,
            column: 0,
            width: 0,
            kind: Internal {
              message: "unexpected end of token stream".to_string(),
            },
          });
        }
      }
    }

    if let Some(token) = self.tokens.next() {
      return Err(token.error(Internal {
        message: format!(
          "unexpected token remaining after parsing completed: {:?}",
          token.kind
        ),
      }));
    }

    AssignmentResolver::resolve_assignments(&self.assignments, &self.assignment_tokens)?;

    RecipeResolver::resolve_recipes(&self.recipes, &self.assignments, self.text)?;

    for recipe in self.recipes.values() {
      for parameter in &recipe.parameters {
        if self.assignments.contains_key(parameter.token.lexeme()) {
          return Err(parameter.token.error(ParameterShadowsVariable {
            parameter: parameter.token.lexeme(),
          }));
        }
      }

      for dependency in &recipe.dependency_tokens {
        if !self.recipes[dependency.lexeme()].parameters.is_empty() {
          return Err(dependency.error(DependencyHasParameters {
            recipe: recipe.name,
            dependency: dependency.lexeme(),
          }));
        }
      }
    }

    AliasResolver::resolve_aliases(&self.aliases, &self.recipes, &self.alias_tokens)?;

    Ok(Justfile {
      recipes: self.recipes,
      assignments: self.assignments,
      exports: self.exports,
      aliases: self.aliases,
      warnings: self.warnings,
    })
  }
}

#[cfg(test)]
mod test {
  use super::*;
  use crate::testing::parse;

  macro_rules! parse_test {
    ($name:ident, $input:expr, $expected:expr $(,)*) => {
      #[test]
      fn $name() {
        let input = $input;
        let expected = $expected;
        let justfile = parse(input);
        let actual = format!("{:#}", justfile);
        use pretty_assertions::assert_eq;
        assert_eq!(actual, expected);
        println!("Re-parsing...");
        let reparsed = parse(&actual);
        let redumped = format!("{:#}", reparsed);
        assert_eq!(redumped, actual);
      }
    };
  }

  parse_test! {
    parse_empty,
    "

# hello


    ",
    "",
  }

  parse_test! {
    parse_string_default,
    r#"

foo a="b\t":


  "#,
    r#"foo a="b\t":"#,
  }

  parse_test! {
  parse_multiple,
    r#"
a:
b:
"#,
    r#"a:

b:"#,
  }

  parse_test! {
    parse_variadic,
    r#"

foo +a:


  "#,
    r#"foo +a:"#,
  }

  parse_test! {
    parse_variadic_string_default,
    r#"

foo +a="Hello":


  "#,
    r#"foo +a="Hello":"#,
  }

  parse_test! {
    parse_raw_string_default,
    r#"

foo a='b\t':


  "#,
    r#"foo a='b\t':"#,
  }

  parse_test! {
    parse_export,
    r#"
export a := "hello"

  "#,
    r#"export a := "hello""#,
  }

  parse_test! {
  parse_alias_after_target,
    r#"
foo:
  echo a
alias f := foo
"#,
r#"alias f := foo

foo:
    echo a"#
  }

  parse_test! {
  parse_alias_before_target,
    r#"
alias f := foo
foo:
  echo a
"#,
r#"alias f := foo

foo:
    echo a"#
  }

  parse_test! {
  parse_alias_with_comment,
    r#"
alias f := foo #comment
foo:
  echo a
"#,
r#"alias f := foo

foo:
    echo a"#
  }

  parse_test! {
  parse_complex,
    "
x:
y:
z:
foo := \"xx\"
bar := foo
goodbye := \"y\"
hello a b    c   : x y    z #hello
  #! blah
  #blarg
  {{ foo + bar}}abc{{ goodbye\t  + \"x\" }}xyz
  1
  2
  3
",
    "bar := foo

foo := \"xx\"

goodbye := \"y\"

hello a b c: x y z
    #! blah
    #blarg
    {{foo + bar}}abc{{goodbye + \"x\"}}xyz
    1
    2
    3

x:

y:

z:"
  }

  parse_test! {
  parse_shebang,
    "
practicum := 'hello'
install:
\t#!/bin/sh
\tif [[ -f {{practicum}} ]]; then
\t\treturn
\tfi
",
    "practicum := 'hello'

install:
    #!/bin/sh
    if [[ -f {{practicum}} ]]; then
    \treturn
    fi",
  }

  parse_test! {
    parse_simple_shebang,
    "a:\n #!\n  print(1)",
    "a:\n    #!\n     print(1)",
  }

  parse_test! {
  parse_assignments,
    r#"a := "0"
c := a + b + a + b
b := "1"
"#,
    r#"a := "0"

b := "1"

c := a + b + a + b"#,
  }

  parse_test! {
  parse_assignment_backticks,
    "a := `echo hello`
c := a + b + a + b
b := `echo goodbye`",
    "a := `echo hello`

b := `echo goodbye`

c := a + b + a + b",
  }

  parse_test! {
  parse_interpolation_backticks,
    r#"a:
  echo {{  `echo hello` + "blarg"   }} {{   `echo bob`   }}"#,
    r#"a:
    echo {{`echo hello` + "blarg"}} {{`echo bob`}}"#,
  }

  parse_test! {
    eof_test,
    "x:\ny:\nz:\na b c: x y z",
    "a b c: x y z\n\nx:\n\ny:\n\nz:",
  }

  parse_test! {
    string_quote_escape,
    r#"a := "hello\"""#,
    r#"a := "hello\"""#,
  }

  parse_test! {
    string_escapes,
    r#"a := "\n\t\r\"\\""#,
    r#"a := "\n\t\r\"\\""#,
  }

  parse_test! {
  parameters,
    "a b c:
  {{b}} {{c}}",
    "a b c:
    {{b}} {{c}}",
  }

  parse_test! {
  unary_functions,
    "
x := arch()

a:
  {{os()}} {{os_family()}}",
    "x := arch()

a:
    {{os()}} {{os_family()}}",
  }

  parse_test! {
  env_functions,
    r#"
x := env_var('foo',)

a:
  {{env_var_or_default('foo' + 'bar', 'baz',)}} {{env_var(env_var("baz"))}}"#,
    r#"x := env_var('foo')

a:
    {{env_var_or_default('foo' + 'bar', 'baz')}} {{env_var(env_var("baz"))}}"#,
  }

  parse_test! {
    parameter_default_string,
    r#"
f x="abc":
"#,
    r#"f x="abc":"#,
  }

  parse_test! {
    parameter_default_raw_string,
    r#"
f x='abc':
"#,
    r#"f x='abc':"#,
  }

  parse_test! {
    parameter_default_backtick,
    r#"
f x=`echo hello`:
"#,
    r#"f x=`echo hello`:"#,
  }

  parse_test! {
    parameter_default_concatination_string,
    r#"
f x=(`echo hello` + "foo"):
"#,
    r#"f x=(`echo hello` + "foo"):"#,
  }

  parse_test! {
    parameter_default_concatination_variable,
    r#"
x := "10"
f y=(`echo hello` + x) +z="foo":
"#,
    r#"x := "10"

f y=(`echo hello` + x) +z="foo":"#,
  }

  parse_test! {
    parameter_default_multiple,
    r#"
x := "10"
f y=(`echo hello` + x) +z=("foo" + "bar"):
"#,
    r#"x := "10"

f y=(`echo hello` + x) +z=("foo" + "bar"):"#,
  }

  parse_test! {
    concatination_in_group,
    "x := ('0' + '1')",
    "x := ('0' + '1')",
  }

  parse_test! {
    string_in_group,
    "x := ('0'   )",
    "x := ('0')",
  }

  #[rustfmt::skip]
  parse_test! {
    escaped_dos_newlines,
    "@spam:\r
\t{ \\\r
\t\tfiglet test; \\\r
\t\tcargo build --color always 2>&1; \\\r
\t\tcargo test  --color always -- --color always 2>&1; \\\r
\t} | less\r
",
"@spam:
    { \\
    \tfiglet test; \\
    \tcargo build --color always 2>&1; \\
    \tcargo test  --color always -- --color always 2>&1; \\
    } | less",
  }

  error_test! {
    name: duplicate_alias,
    input: "alias foo = bar\nalias foo = baz",
    offset: 22,
    line: 1,
    column: 6,
    width: 3,
    kind: DuplicateAlias { alias: "foo", first: 0 },
  }

  error_test! {
    name: alias_syntax_multiple_rhs,
    input: "alias foo = bar baz",
    offset: 16,
    line: 0,
    column: 16,
    width: 3,
    kind: UnexpectedToken { expected: vec![Eol, Eof], found: Name },
  }

  error_test! {
    name: alias_syntax_no_rhs,
    input: "alias foo = \n",
    offset: 12,
    line: 0,
    column: 12,
    width: 1,
    kind: UnexpectedToken {expected: vec![Name], found:Eol},
  }

  error_test! {
    name: unknown_alias_target,
    input: "alias foo = bar\n",
    offset: 6,
    line: 0,
    column: 6,
    width: 3,
    kind: UnknownAliasTarget {alias: "foo", target: "bar"},
  }

  error_test! {
    name: alias_shadows_recipe_before,
    input: "bar: \n  echo bar\nalias foo = bar\nfoo:\n  echo foo",
    offset: 23,
    line: 2,
    column: 6,
    width: 3,
    kind: AliasShadowsRecipe {alias: "foo", recipe_line: 3},
  }

  error_test! {
    name: alias_shadows_recipe_after,
    input: "foo:\n  echo foo\nalias foo = bar\nbar:\n  echo bar",
    offset: 22,
    line: 2,
    column: 6,
    width: 3,
    kind: AliasShadowsRecipe { alias: "foo", recipe_line: 0 },
  }

  error_test! {
    name:   missing_colon,
    input:  "a b c\nd e f",
    offset:  5,
    line:   0,
    column: 5,
    width:  1,
    kind:   UnexpectedToken{expected: vec![Name, Plus, Colon], found: Eol},
  }

  error_test! {
    name:   missing_default_eol,
    input:  "hello arg=\n",
    offset:  10,
    line:   0,
    column: 10,
    width:  1,
    kind:   UnexpectedToken{expected: vec![Name, StringCooked], found: Eol},
  }

  error_test! {
    name:   missing_default_eof,
    input:  "hello arg=",
    offset:  10,
    line:   0,
    column: 10,
    width:  0,
    kind:   UnexpectedToken{expected: vec![Name, StringCooked], found: Eof},
  }

  error_test! {
    name:   parameter_after_variadic,
    input:  "foo +a bbb:",
    offset:  7,
    line:   0,
    column: 7,
    width:  3,
    kind:   ParameterFollowsVariadicParameter{parameter: "bbb"},
  }

  error_test! {
    name:   required_after_default,
    input:  "hello arg='foo' bar:",
    offset:  16,
    line:   0,
    column: 16,
    width:  3,
    kind:   RequiredParameterFollowsDefaultParameter{parameter: "bar"},
  }

  error_test! {
    name:   missing_eol,
    input:  "a b c: z =",
    offset:  9,
    line:   0,
    column: 9,
    width:  1,
    kind:   UnexpectedToken{expected: vec![Name, Eol, Eof], found: Equals},
  }

  error_test! {
    name:   duplicate_parameter,
    input:  "a b b:",
    offset:  4,
    line:   0,
    column: 4,
    width:  1,
    kind:   DuplicateParameter{recipe: "a", parameter: "b"},
  }

  error_test! {
    name:   parameter_shadows_varible,
    input:  "foo = \"h\"\na foo:",
    offset:  12,
    line:   1,
    column: 2,
    width:  3,
    kind:   ParameterShadowsVariable{parameter: "foo"},
  }

  error_test! {
    name:   dependency_has_parameters,
    input:  "foo arg:\nb: foo",
    offset:  12,
    line:   1,
    column: 3,
    width:  3,
    kind:   DependencyHasParameters{recipe: "b", dependency: "foo"},
  }

  error_test! {
    name:   duplicate_dependency,
    input:  "a b c: b c z z",
    offset:  13,
    line:   0,
    column: 13,
    width:  1,
    kind:   DuplicateDependency{recipe: "a", dependency: "z"},
  }

  error_test! {
    name:   duplicate_recipe,
    input:  "a:\nb:\na:",
    offset:  6,
    line:   2,
    column: 0,
    width:  1,
    kind:   DuplicateRecipe{recipe: "a", first: 0},
  }

  error_test! {
    name:   duplicate_variable,
    input:  "a = \"0\"\na = \"0\"",
    offset:  8,
    line:   1,
    column: 0,
    width:  1,
    kind:   DuplicateVariable{variable: "a"},
  }

  error_test! {
    name:   extra_whitespace,
    input:  "a:\n blah\n  blarg",
    offset:  10,
    line:   2,
    column: 1,
    width:  6,
    kind:   ExtraLeadingWhitespace,
  }

  error_test! {
    name:   interpolation_outside_of_recipe,
    input:  "{{",
    offset:  0,
    line:   0,
    column: 0,
    width:  2,
    kind:   UnexpectedToken{expected: vec![Name, At], found: InterpolationStart},
  }

  error_test! {
    name:   unclosed_parenthesis_in_expression,
    input:  "x = foo(",
    offset:  8,
    line:   0,
    column: 8,
    width:  0,
    kind:   UnexpectedToken{expected: vec![Name, StringCooked, ParenR], found: Eof},
  }

  error_test! {
    name:   unclosed_parenthesis_in_interpolation,
    input:  "a:\n echo {{foo(}}",
    offset:  15,
    line:   1,
    column: 12,
    width:  2,
    kind:   UnexpectedToken{expected: vec![Name, StringCooked, ParenR], found: InterpolationEnd},
  }

  error_test! {
    name:   plus_following_parameter,
    input:  "a b c+:",
    offset:  5,
    line:   0,
    column: 5,
    width:  1,
    kind:   UnexpectedToken{expected: vec![Name], found: Plus},
  }

  error_test! {
    name:   bad_export,
    input:  "export a",
    offset:  8,
    line:   0,
    column: 8,
    width:  0,
    kind:   UnexpectedToken{expected: vec![Name, Plus, Colon], found: Eof},
  }

  #[test]
  fn readme_test() {
    let mut justfiles = vec![];
    let mut current = None;

    for line in fs::read_to_string("README.adoc").unwrap().lines() {
      if let Some(mut justfile) = current {
        if line == "```" {
          justfiles.push(justfile);
          current = None;
        } else {
          justfile += line;
          justfile += "\n";
          current = Some(justfile);
        }
      } else if line == "```make" {
        current = Some(String::new());
      }
    }

    for justfile in justfiles {
      parse(&justfile);
    }
  }

  #[test]
  fn empty_recipe_lines() {
    let text = "a:";
    let justfile = parse(&text);

    assert_eq!(justfile.recipes["a"].lines.len(), 0);
  }

  #[test]
  fn simple_recipe_lines() {
    let text = "a:\n foo";
    let justfile = parse(&text);

    assert_eq!(justfile.recipes["a"].lines.len(), 1);
  }

  #[test]
  fn complex_recipe_lines() {
    let text = "a:
  foo

b:
";

    let justfile = parse(&text);

    assert_eq!(justfile.recipes["a"].lines.len(), 1);
  }
}
