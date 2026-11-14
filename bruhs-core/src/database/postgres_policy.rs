//! Parse before PostgreSQL prepares anything: even SELECT functions can perform external writes.
//! Fail closed on callable/schema features beyond this browser's supported SELECT subset.
use super::DatabaseError;
use sqlparser::{ast::*, dialect::PostgreSqlDialect, parser::Parser};
use std::{collections::BTreeSet, ops::ControlFlow};

pub(super) struct CheckedQuery {
    pub sql: String,
    pub relations: BTreeSet<(String, String)>,
}
struct Policy {
    depth: usize,
    relations: BTreeSet<(String, String)>,
}
fn denied() -> DatabaseError {
    DatabaseError::Invalid("This Postgres browser supports SELECT over ordinary tables with built-in expressions. Arbitrary functions, views, RLS, custom types and SQL commands are blocked because they can hide writes. Use the schema inspector for metadata.".into())
}
fn parts(name: &ObjectName) -> Option<Vec<String>> {
    name.0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(id) => Some(if id.quote_style.is_some() {
                id.value.clone()
            } else {
                id.value.to_ascii_lowercase()
            }),
            _ => None,
        })
        .collect()
}
fn builtin(name: &ObjectName) -> bool {
    let Some(parts) = parts(name) else {
        return false;
    };
    let function = match parts.as_slice() {
        [function] => function,
        [schema, function] if schema == "pg_catalog" => function,
        _ => return false,
    };
    matches!(
        function.as_str(),
        "count"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "lower"
            | "upper"
            | "length"
            | "char_length"
            | "octet_length"
            | "trim"
            | "btrim"
            | "ltrim"
            | "rtrim"
            | "replace"
            | "substring"
            | "substr"
            | "strpos"
            | "concat"
            | "concat_ws"
            | "coalesce"
            | "nullif"
            | "greatest"
            | "least"
            | "abs"
            | "round"
            | "floor"
            | "ceil"
            | "ceiling"
            | "date_part"
            | "date_trunc"
            | "now"
            | "pg_sleep"
            | "repeat"
            | "generate_series"
    )
}
fn primitive(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Bool
            | DataType::Boolean
            | DataType::SmallInt(_)
            | DataType::Int2(_)
            | DataType::Int(_)
            | DataType::Integer(_)
            | DataType::Int4(_)
            | DataType::Int8(_)
            | DataType::BigInt(_)
            | DataType::Real
            | DataType::Float4
            | DataType::Float8
            | DataType::Double(_)
            | DataType::DoublePrecision
            | DataType::Text
            | DataType::Varchar(_)
            | DataType::CharacterVarying(_)
            | DataType::Numeric(_)
            | DataType::Decimal(_)
            | DataType::Date
            | DataType::Time(..)
            | DataType::Timestamp(..)
            | DataType::Uuid
            | DataType::JSON
            | DataType::JSONB
            | DataType::Bytea
    )
}
// Matches sqlparser 0.64's default remaining parser depth. Apply it to our AST visitor
// too: long left-associated expressions/unions need not recurse deeply while parsing.
const MAX_AST_DEPTH: usize = 50;
impl Policy {
    fn enter(&mut self) -> ControlFlow<DatabaseError> {
        if self.depth >= MAX_AST_DEPTH {
            ControlFlow::Break(denied())
        } else {
            self.depth += 1;
            ControlFlow::Continue(())
        }
    }
}
fn check_set(root: &SetExpr) -> bool {
    let mut pending = vec![(root, 0)];
    while let Some((set, depth)) = pending.pop() {
        if depth >= MAX_AST_DEPTH {
            return false;
        }
        match set {
            SetExpr::Select(_) | SetExpr::Values(_) => {}
            SetExpr::Query(query) => pending.push((&query.body, depth + 1)),
            SetExpr::SetOperation { left, right, .. } => {
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            _ => return false,
        }
    }
    true
}

impl VisitorMut for Policy {
    type Break = DatabaseError;
    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<Self::Break> {
        self.enter()?;
        if !check_set(&query.body) {
            return ControlFlow::Break(denied());
        }
        if query.with.is_some()
            || !query.locks.is_empty()
            || query.for_clause.is_some()
            || query.settings.is_some()
            || query.format_clause.is_some()
            || !query.pipe_operators.is_empty()
            || !matches!(
                query.body.as_ref(),
                SetExpr::Select(_)
                    | SetExpr::Query(_)
                    | SetExpr::SetOperation { .. }
                    | SetExpr::Values(_)
            )
        {
            return ControlFlow::Break(denied());
        }
        ControlFlow::Continue(())
    }
    fn post_visit_query(&mut self, _: &mut Query) -> ControlFlow<Self::Break> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }
    fn post_visit_expr(&mut self, _: &mut Expr) -> ControlFlow<Self::Break> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }
    fn post_visit_table_factor(&mut self, _: &mut TableFactor) -> ControlFlow<Self::Break> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }
    fn pre_visit_statement(&mut self, statement: &mut Statement) -> ControlFlow<Self::Break> {
        if matches!(statement, Statement::Query(_)) {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(denied())
        }
    }
    fn pre_visit_select(&mut self, select: &mut Select) -> ControlFlow<Self::Break> {
        if select.into.is_some()
            || !select.lateral_views.is_empty()
            || !select.optimizer_hints.is_empty()
            || !select.connect_by.is_empty()
        {
            return ControlFlow::Break(denied());
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_table_factor(&mut self, table: &mut TableFactor) -> ControlFlow<Self::Break> {
        self.enter()?;
        match table {
            TableFactor::Table {
                name,
                args,
                with_hints,
                version,
                partitions,
                json_path,
                sample,
                index_directives,
                ..
            } => {
                if !with_hints.is_empty()
                    || version.is_some()
                    || !partitions.is_empty()
                    || json_path.is_some()
                    || sample.is_some()
                    || !index_directives.is_empty()
                {
                    return ControlFlow::Break(denied());
                }
                if args.is_some() {
                    return if builtin(name) {
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(denied())
                    };
                }
                let Some(components) = parts(name) else {
                    return ControlFlow::Break(denied());
                };
                let (schema, relation) = match components.as_slice() {
                    [relation] => ("public".to_string(), relation.clone()),
                    [schema, relation] => (schema.clone(), relation.clone()),
                    _ => return ControlFlow::Break(denied()),
                };
                self.relations.insert((schema.clone(), relation.clone()));
                *name = ObjectName::from(vec![
                    Ident::with_quote('"', schema),
                    Ident::with_quote('"', relation),
                ]);
            }
            TableFactor::Derived { sample: None, .. } | TableFactor::NestedJoin { .. } => {}
            TableFactor::Function { name, .. } if builtin(name) => {}
            _ => return ControlFlow::Break(denied()),
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        self.enter()?;
        let allowed = match expr {
            Expr::Identifier(_)
            | Expr::CompoundIdentifier(_)
            | Expr::Value(_)
            | Expr::Nested(_)
            | Expr::IsNull(_)
            | Expr::IsNotNull(_)
            | Expr::IsTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsNotTrue(_)
            | Expr::IsNotFalse(_)
            | Expr::IsDistinctFrom(..)
            | Expr::IsNotDistinctFrom(..)
            | Expr::Between { .. }
            | Expr::Like { .. }
            | Expr::ILike { .. }
            | Expr::Case { .. }
            | Expr::Subquery(_)
            | Expr::Exists { .. }
            | Expr::Tuple(_)
            | Expr::Position { .. }
            | Expr::Substring { .. }
            | Expr::Trim { .. }
            | Expr::Extract { .. }
            | Expr::AtTimeZone { .. } => true,
            Expr::In { target, .. } => matches!(target, InTarget::List(_) | InTarget::Subquery(_)),
            Expr::BinaryOp { op, .. } => matches!(
                op,
                BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo
                    | BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::And
                    | BinaryOperator::Or
                    | BinaryOperator::StringConcat
            ),
            Expr::UnaryOp { op, .. } => matches!(
                op,
                UnaryOperator::Plus | UnaryOperator::Minus | UnaryOperator::Not
            ),
            Expr::Function(function) => builtin(&function.name),
            Expr::Cast {
                data_type,
                format: None,
                ..
            } => primitive(data_type),
            Expr::TypedString(value) => primitive(&value.data_type),
            _ => false,
        };
        if allowed {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(denied())
        }
    }
}
pub(super) fn check(sql: &str) -> Result<CheckedQuery, DatabaseError> {
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).map_err(|_| denied())?;
    if statements.len() != 1 {
        return Err(denied());
    }
    let mut policy = Policy {
        depth: 0,
        relations: BTreeSet::new(),
    };
    let statement = &mut statements[0];
    if let ControlFlow::Break(error) = statement.visit(&mut policy) {
        return Err(error);
    }
    Ok(CheckedQuery {
        sql: statement.to_string(),
        relations: policy.relations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parsed_sql_rejects_hidden_commands_functions_types_and_operators() {
        for sql in [
            "SELECT public.dblink_exec('x','UPDATE customers SET name=1')",
            "SELECT pg_catalog.query_to_xml('DELETE FROM customers RETURNING *',true,false,'')",
            "SELECT pg_catalog.pg_logical_emit_message(false,'x','data')",
            "SELECT pg_catalog.set_config('transaction_read_only','off',true)",
            "SELECT 'x'::public.bad",
            "SELECT 1 OPERATOR(public.+) 2",
            "SELECT * INTO bad FROM customers",
            "WITH x AS (DELETE FROM customers RETURNING *) SELECT * FROM x",
            "SELECT 1;SELECT 2",
            "SELECT * FROM public.dblink('x','x') AS x(id int)",
        ] {
            assert!(check(sql).is_err(), "allowed {sql}");
        }
        let checked = check(
            "SELECT id,lower(name) FROM customers WHERE subscription IS NULL ORDER BY id LIMIT 10",
        )
        .unwrap();
        assert!(checked.sql.contains("\"public\".\"customers\""));
        assert_eq!(checked.relations.len(), 1);
    }
}
