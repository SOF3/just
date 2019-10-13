struct Justfile<'text> {
  items: Vec<Item<'text>>,
}

enum Item<'text> {
  Recipe(Recipe<'text>),
  Alias(Alias<'text>),
  Assignment(Assignment<'text>),
}

struct Recipe<'text> {
  name: &'text str,
  parameters: Vec<Parameter<'text>>,
  body: Vec<Line<'text>>,
}

struct Line<'text> {
  fragments: Vec<Fragment<'text>>,
}

enum Fragment<'text> {
  Text(&'text str),
  Interoplation(Expression<'text>),
}

enum Expression<'text> {
  Value(Value<'text>),
  Addition(Value<'text>, Box<Expression<'text>>),
}

enum Value<'text> {
  Call {
    name: &'text str,
    arguments: Vec<Expression<'text>>,
  },
}

struct Parameter<'text> {
  name: &'text str,
  default: Value<'text>,
}

struct Alias<'text> {
  name: &'text str,
  target: &'text str,
}

struct Assignment<'text> {
  export: bool,
  name: &'text str,
}
