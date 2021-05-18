use graphql_parser::query::parse_query;
use graphql_parser::query::Definition::Fragment;
use graphql_parser::query::Definition::Operation;
use graphql_parser::query::FragmentDefinition;
use graphql_parser::query::OperationDefinition;
use graphql_parser::query::Selection::{Field, FragmentSpread, InlineFragment};
use graphql_parser::query::SelectionSet;
use graphql_parser::query::TypeCondition::On;
use graphql_parser::schema::{parse_schema, Definition, TypeDefinition};
use graphql_parser::schema::{InterfaceType, ObjectType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::read_to_string;

#[derive(Debug)]
struct Stats {
    name: String,
    age: u8,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StatsField {
    name: String,
    pub r#type: String,
    pub count: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GraphQLType {
    pub name: String,
    pub fields: HashMap<String, StatsField>,
}

impl GraphQLType {
    pub fn from_object(obj: ObjectType<String>) -> Self {
        let mut fields = HashMap::new();

        for field in obj.fields {
            fields.insert(
                field.name.clone(),
                StatsField {
                    name: field.name,
                    r#type: field.field_type.to_string(),
                    count: 0,
                },
            );
        }
        GraphQLType {
            name: obj.name,
            fields: fields,
        }
    }

    pub fn from_interface(obj: InterfaceType<String>) -> Self {
        let mut fields = HashMap::new();

        for field in obj.fields {
            fields.insert(
                field.name.clone(),
                StatsField {
                    name: field.name,
                    r#type: field.field_type.to_string(),
                    count: 0,
                },
            );
        }
        GraphQLType {
            name: obj.name,
            fields: fields,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Query {
    pub query: String,
    pub hash: String,
    pub count: i32,
}

fn get_schema_types(schema: String) -> HashMap<String, GraphQLType> {
    let mut types: HashMap<String, GraphQLType> = HashMap::new();
    let ast = parse_schema::<String>(&schema).expect("Unable to parse schema");

    for definition in ast.definitions {
        match definition {
            Definition::TypeDefinition(t) => match t {
                TypeDefinition::Object(o) => {
                    let type_ = GraphQLType::from_object(o);

                    types.insert(type_.name.clone(), type_);
                }
                TypeDefinition::Interface(i) => {
                    let type_ = GraphQLType::from_interface(i);

                    types.insert(type_.name.clone(), type_);
                }
                _ => {}
            },
            _ => {}
        }
    }

    return types;
}

fn get_tree_for_selection_set<'a>(
    selection_set: SelectionSet<'a, &'a str>,
    root_type_name: &str,
    schema: &mut HashMap<String, GraphQLType>,
    fragments: &HashMap<String, FragmentDefinition<'a, &'a str>>,
) -> Result<(), String> {
    for item in selection_set.items {
        match item {
            Field(f) => {
                if f.name == "__typename" {
                    continue;
                }

                if !schema.contains_key(root_type_name) {
                    return Err(format!("Unable to get type {}", root_type_name));
                }

                let root_type = schema.get_mut(root_type_name).unwrap();

                if !root_type.fields.contains_key(f.name) {
                    return Err(format!("Unable to get {} in {}", f.name, root_type_name));
                }

                // let &mut field: StatsField = root_type.fields.get(f.name).unwrap();
                let field = root_type.fields.get_mut(f.name).unwrap();

                field.count += 1;

                let field_type_name = field
                    .r#type
                    .clone()
                    .replace("[", "")
                    .replace("]", "")
                    .replace("!", "");

                if let Err(err) =
                    get_tree_for_selection_set(f.selection_set, &field_type_name, schema, fragments)
                {
                    return Err(err);
                }
            }
            InlineFragment(fragment) => {
                let type_condition = fragment.type_condition.unwrap();

                match type_condition {
                    On(type_name) => {
                        if let Err(err) = get_tree_for_selection_set(
                            fragment.selection_set,
                            &type_name,
                            schema,
                            fragments,
                        ) {
                            return Err(err);
                        }
                    }
                }
            }
            FragmentSpread(fragment_spread) => {
                if fragments.contains_key(fragment_spread.fragment_name) {
                    let fragment = &fragments[fragment_spread.fragment_name];

                    match &fragment.type_condition {
                        On(type_name) => {
                            if let Err(err) = get_tree_for_selection_set(
                                fragment.selection_set.clone(),
                                &type_name,
                                schema,
                                fragments,
                            ) {
                                return Err(err);
                            }
                        }
                    }
                }
            }
        }
    }

    return Ok(());
}

fn update_usages_for_operation<'a>(
    operation: OperationDefinition<'a, &'a str>,
    schema: &mut HashMap<String, GraphQLType>,
    fragments: &HashMap<String, FragmentDefinition<'a, &'a str>>,
) -> Result<(), String> {
    match operation {
        OperationDefinition::Query(q) => {
            return get_tree_for_selection_set(q.selection_set, "Query", schema, fragments)
        }
        OperationDefinition::Mutation(m) => {
            return get_tree_for_selection_set(m.selection_set, "Mutation", schema, fragments)
        }
        OperationDefinition::Subscription(s) => {
            return get_tree_for_selection_set(s.selection_set, "Subscription", schema, fragments)
        }
        // this is when no operation this is passed, which means we have a query
        OperationDefinition::SelectionSet(selection_set) => {
            return get_tree_for_selection_set(selection_set, "Query", schema, fragments)
        }
    }
}

fn update_usages_for_fragment<'a>(
    fragment: &FragmentDefinition<'a, &'a str>,
    schema: &mut HashMap<String, GraphQLType>,
    fragments: &HashMap<String, FragmentDefinition<'a, &'a str>>,
) -> Result<(), String> {
    match fragment.type_condition {
        On(type_name) => get_tree_for_selection_set(
            fragment.selection_set.clone(),
            &type_name,
            schema,
            fragments,
        ),
    }
}

fn extract_queries_and_fragments<'a>(
    document: &'a str,
) -> (
    Vec<OperationDefinition<'a, &str>>,
    HashMap<String, FragmentDefinition<'a, &str>>,
) {
    let ast = parse_query::<&str>(document).expect("Unable to parse queries");

    let mut operations: Vec<OperationDefinition<&str>> = Vec::new();
    let mut fragments: HashMap<String, FragmentDefinition<&str>> = HashMap::new();

    for definition in ast.definitions {
        match definition {
            Operation(o) => {
                operations.push(o);
            }
            Fragment(f) => {
                fragments.insert(f.name.to_string(), f);
            }
        }
    }

    return (operations, fragments);
}

pub fn generate_schema_stats(
    schema_path: String,
    queries_document: String,
    include_fragments: bool,
) -> HashMap<String, GraphQLType> {
    let sdl = read_to_string(schema_path).expect("Unable to read schema");

    let mut schema = get_schema_types(sdl);

    let (queries, fragments) = extract_queries_and_fragments(&queries_document);

    for operation in queries {
        if let Err(err) = update_usages_for_operation(operation, &mut schema, &fragments) {
            eprintln!("Error: {}", err)
        }
    }

    if include_fragments {
        for (_, fragment) in &fragments {
            if let Err(err) = update_usages_for_fragment(fragment, &mut schema, &fragments) {
                eprintln!("Fragment error: {}", err)
            }
        }
    }

    return schema;
}
