//! Intentionally narrow equivalence of explicit built-in method headers.
//! This is neither inference nor a typecheck of method bodies or call receivers.
use crate::IssueKind;
use crate::ast::Language;
use crate::incremental::Budget;
use tree_sitter::Node;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MethodContract {
    pub(crate) visibility: String,
    static_member: bool,
    parameters: Vec<ParameterContract>,
    generics: Vec<(Option<TypeContract>, Option<TypeContract>)>,
    returns: TypeContract,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParameterContract {
    name: String,
    ty: TypeContract,
    optional: bool,
    rest: bool,
    default: Option<String>,
}

// Compare only literal defaults whose meaning is independent of namespace,
// evaluation order or a runtime call. Exact spelling is deliberately retained.
fn literal_default(
    node: Node<'_>,
    source: &str,
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    budget.node()?;
    match node.kind() {
        "integer" | "float" | "number" | "boolean" | "true" | "false" | "null" => Ok(true),
        "string" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                budget.node()?;
                if !matches!(
                    child.kind(),
                    "string_fragment" | "string_content" | "escape_sequence"
                ) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        "unary_expression" | "unary_op_expression" => {
            let value = node.child_by_field_name("argument");
            Ok(
                value.is_some_and(|n| matches!(n.kind(), "number" | "integer" | "float"))
                    && matches!(source[node.byte_range()].chars().next(), Some('+' | '-')),
            )
        }
        _ => Ok(false),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum TypeContract {
    Primitive(&'static str),
    Union(Vec<TypeContract>),
    Array(Box<TypeContract>),
    GenericParameter(usize),
    Nominal(String),
}
pub(crate) fn explicit_type(
    node: Node<'_>,
    source: &str,
    lang: Language,
    returns: bool,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<Option<TypeContract>, IssueKind> {
    budget.node()?;
    if depth > 16 {
        return Ok(None);
    }
    let mut cursor = node.walk();
    let children = node
        .named_children(&mut cursor)
        .filter(|n| n.kind() != "comment")
        .collect::<Vec<_>>();
    if lang == Language::TypeScript
        && matches!(node.kind(), "type_annotation" | "parenthesized_type")
    {
        return if children.len() == 1 {
            explicit_type(children[0], source, lang, returns, budget, depth + 1)
        } else {
            Ok(None)
        };
    }
    if node.kind() == "union_type" || (lang == Language::Php && node.kind() == "optional_type") {
        let nullable = node.kind() == "optional_type";
        if (nullable && children.len() != 1) || (!nullable && children.len() < 2) {
            return Ok(None);
        }
        let mut members = vec![];
        for child in children {
            let Some(ty) = explicit_type(child, source, lang, false, budget, depth + 1)? else {
                return Ok(None);
            };
            match ty {
                TypeContract::Union(types) => members.extend(types),
                ty => members.push(ty),
            }
        }
        if nullable {
            members.push(TypeContract::Primitive("null"));
        }
        members.sort();
        // Duplicate PHP union members are invalid; rejecting duplicates in both
        // languages keeps the proof conservative without relying on type reduction.
        if members.windows(2).any(|v| v[0] == v[1]) {
            return Ok(None);
        }
        return Ok(Some(TypeContract::Union(members)));
    }
    if lang == Language::TypeScript && node.kind() == "array_type" && children.len() == 1 {
        return Ok(
            explicit_type(children[0], source, lang, false, budget, depth + 1)?
                .map(|ty| TypeContract::Array(Box::new(ty))),
        );
    }
    if lang == Language::TypeScript && node.kind() == "type_identifier" {
        let name = &source[node.byte_range()];
        let mut ancestor = node.parent();
        while let Some(owner) = ancestor {
            budget.node()?;
            if let Some(parameters) = owner.child_by_field_name("type_parameters") {
                let mut cursor = parameters.walk();
                for (index, parameter) in parameters
                    .named_children(&mut cursor)
                    .filter(|n| n.kind() == "type_parameter")
                    .enumerate()
                {
                    budget.node()?;
                    if parameter
                        .child_by_field_name("name")
                        .is_some_and(|n| &source[n.byte_range()] == name)
                    {
                        return if matches!(
                            owner.kind(),
                            "method_definition" | "method_signature" | "abstract_method_signature"
                        ) {
                            Ok(Some(TypeContract::GenericParameter(index)))
                        } else {
                            Ok(None)
                        };
                    }
                }
            }
            ancestor = owner.parent();
        }
        // Local aliases in ES modules cannot merge with another file's global
        // declarations. Imported aliases and namespace/class generics require a
        // separate module/type resolver and are deliberately not guessed here.
        let mut root = node;
        while let Some(parent) = root.parent() {
            root = parent;
        }
        let mut cursor = root.walk();
        let declarations = root.named_children(&mut cursor).collect::<Vec<_>>();
        if !declarations
            .iter()
            .any(|n| matches!(n.kind(), "import_statement" | "export_statement"))
        {
            return Ok(None);
        }
        let mut matching = Vec::new();
        for declaration in declarations {
            budget.node()?;
            let declaration = if declaration.kind() == "export_statement" {
                declaration
                    .child_by_field_name("declaration")
                    .unwrap_or(declaration)
            } else {
                declaration
            };
            if declaration
                .child_by_field_name("name")
                .is_some_and(|n| &source[n.byte_range()] == name)
            {
                matching.push(declaration);
            }
            if declaration.kind() == "import_statement" {
                let mut stack = vec![declaration];
                while let Some(part) = stack.pop() {
                    budget.node()?;
                    if matches!(part.kind(), "identifier" | "type_identifier")
                        && &source[part.byte_range()] == name
                    {
                        return Ok(None);
                    }
                    let mut cursor = part.walk();
                    stack.extend(part.named_children(&mut cursor));
                }
            }
        }
        if matching.len() == 1
            && matching[0].kind() == "type_alias_declaration"
            && matching[0].child_by_field_name("type_parameters").is_none()
            && let Some(value) = matching[0].child_by_field_name("value")
        {
            return explicit_type(value, source, lang, returns, budget, depth + 1);
        }
        return Ok(None);
    }
    if lang == Language::Php && node.kind() == "named_type" {
        return Ok(
            crate::hierarchy::php_type_identity(node, source, budget)?.map(TypeContract::Nominal)
        );
    }
    let atom = match (
        lang,
        node.kind(),
        source[node.byte_range()].to_ascii_lowercase().as_str(),
    ) {
        (Language::Php, "primitive_type", "string") => Some("string"),
        (Language::Php, "primitive_type", "int") => Some("int"),
        (Language::Php, "primitive_type", "float") => Some("float"),
        (Language::Php, "primitive_type", "bool") => Some("bool"),
        (Language::Php, "primitive_type", "array") => Some("array"),
        (Language::Php, "primitive_type", "null") => Some("null"),
        (Language::Php, "primitive_type", "void") if returns => Some("void"),
        (Language::TypeScript, "predefined_type", "string") => Some("string"),
        (Language::TypeScript, "predefined_type", "number") => Some("number"),
        (Language::TypeScript, "predefined_type", "boolean") => Some("boolean"),
        (Language::TypeScript, "literal_type", "undefined") => Some("undefined"),
        (Language::TypeScript, "literal_type", "null") => Some("null"),
        (Language::TypeScript, "predefined_type", "void") if returns => Some("void"),
        _ => None,
    };
    Ok(atom.map(TypeContract::Primitive))
}

pub(crate) fn inspect(
    method: Node<'_>,
    source: &str,
    lang: Language,
    budget: &mut Budget<'_>,
) -> Result<Result<MethodContract, &'static str>, IssueKind> {
    let unsupported = |reason| Ok(Err(reason));
    let interface = method
        .parent()
        .and_then(|n| n.parent())
        .is_some_and(|n| n.kind() == "interface_declaration");
    let Some(parameters) = method.child_by_field_name("parameters") else {
        return unsupported("Parâmetros ausentes");
    };
    let Some(return_node) = method.child_by_field_name("return_type") else {
        return unsupported("Retorno explícito obrigatório para override integralmente verificado");
    };
    let Some(returns) = explicit_type(return_node, source, lang, true, budget, 0)? else {
        return unsupported("Retorno exige tipo explícito suportado (primitivo/union/array)");
    };
    let mut contract = MethodContract {
        visibility: "public".into(),
        static_member: false,
        parameters: vec![],
        generics: vec![],
        returns,
    };
    if let Some(parameters) = method.child_by_field_name("type_parameters") {
        if lang != Language::TypeScript {
            return unsupported("Genéricos fora do TS não resolvidos");
        }
        let mut cursor = parameters.walk();
        let mut names = std::collections::HashSet::new();
        for parameter in parameters.named_children(&mut cursor) {
            budget.node()?;
            if parameter.kind() == "comment" {
                continue;
            }
            let Some(name) = parameter.child_by_field_name("name") else {
                return unsupported("Parâmetro genérico sem nome");
            };
            if !names.insert(&source[name.byte_range()]) {
                return unsupported("Parâmetro genérico duplicado");
            }
            let mut types = Vec::new();
            for field in ["constraint", "value"] {
                let ty = if let Some(wrapper) = parameter.child_by_field_name(field) {
                    let Some(value) = wrapper.named_child(0) else {
                        return unsupported("Contrato genérico vazio");
                    };
                    let Some(ty) = explicit_type(value, source, lang, false, budget, 0)? else {
                        return unsupported("Constraint/default genérico não resolvido");
                    };
                    // No recursive or F-bounded constraints without a type solver.
                    fn has_parameter(t: &TypeContract) -> bool {
                        match t {
                            TypeContract::GenericParameter(_) => true,
                            TypeContract::Array(t) => has_parameter(t),
                            TypeContract::Union(v) => v.iter().any(has_parameter),
                            _ => false,
                        }
                    }
                    if has_parameter(&ty) {
                        return unsupported(
                            "Constraint/default recursivo exige resolução genérica",
                        );
                    }
                    Some(ty)
                } else {
                    None
                };
                types.push(ty);
            }
            let mut children = parameter.walk();
            for child in parameter.children(&mut children) {
                budget.node()?;
                if child != name
                    && !["constraint", "value"]
                        .iter()
                        .any(|field| parameter.child_by_field_name(field) == Some(child))
                    && child.kind() != "comment"
                {
                    return unsupported("Variância/modificador genérico não verificado");
                }
            }
            contract.generics.push((types.remove(0), types.remove(0)));
        }
    }
    let mut cursor = method.walk();
    for node in method.children(&mut cursor) {
        budget.node()?;
        if interface
            && lang == Language::TypeScript
            && matches!(
                node.kind(),
                "accessibility_modifier" | "override_modifier" | "static"
            )
        {
            return unsupported(
                "Interface TypeScript não admite modificadores de método de classe",
            );
        }
        if [
            "name",
            "parameters",
            "return_type",
            "body",
            "type_parameters",
        ]
        .iter()
        .any(|field| method.child_by_field_name(field) == Some(node))
            || matches!(
                node.kind(),
                "comment"
                    | "function"
                    | ":"
                    | "override_modifier"
                    | "abstract"
                    | "abstract_modifier"
            )
            || ((interface
                || method.kind() == "abstract_method_signature"
                || method.child_by_field_name("body").is_none())
                && node.kind() == ";")
        {
            continue;
        }
        match node.kind() {
            "static" | "static_modifier" => contract.static_member = true,
            "visibility_modifier" | "accessibility_modifier" => {
                let visibility = source[node.byte_range()].to_ascii_lowercase();
                if !matches!(visibility.as_str(), "public" | "protected") {
                    return unsupported("Visibilidade private/assimétrica exige análise adicional");
                }
                contract.visibility = visibility;
            }
            _ => {
                return unsupported(
                    "Modificador, genérico, referência, async/generator ou atributo não verificado",
                );
            }
        }
    }
    if interface
        && (contract.visibility != "public"
            || (lang == Language::TypeScript && contract.static_member))
    {
        return unsupported("Interface exige método público de instância no TS");
    }
    let mut parameter_cursor = parameters.walk();
    for parameter in parameters.named_children(&mut parameter_cursor) {
        budget.node()?;
        if parameter.kind() == "comment" {
            continue;
        }
        if !matches!(
            (lang, parameter.kind()),
            (Language::Php, "simple_parameter" | "variadic_parameter")
                | (
                    Language::TypeScript,
                    "required_parameter" | "optional_parameter"
                )
        ) {
            return unsupported("Parâmetro promovido ou padrão composto não verificado");
        }
        let pattern = parameter
            .child_by_field_name("name")
            .or_else(|| parameter.child_by_field_name("pattern"));
        let rest = parameter.kind() == "variadic_parameter"
            || pattern.is_some_and(|n| n.kind() == "rest_pattern");
        let name = pattern.map(|n| {
            if n.kind() == "rest_pattern" {
                n.named_child(0).unwrap_or(n)
            } else {
                n
            }
        });
        let Some(name) = name.filter(|n| {
            matches!(
                (lang, n.kind()),
                (Language::Php, "variable_name") | (Language::TypeScript, "identifier")
            )
        }) else {
            return unsupported("Parâmetro sem identificador simples");
        };
        let Some(ty) = parameter.child_by_field_name("type") else {
            return unsupported("Parâmetro exige tipo explícito suportado");
        };
        let Some(ty_name) = explicit_type(ty, source, lang, false, budget, 0)? else {
            return unsupported(
                "Tipo de parâmetro exige resolução além dos tipos internos suportados",
            );
        };
        let default = parameter
            .child_by_field_name("default_value")
            .or_else(|| parameter.child_by_field_name("value"));
        let optional = parameter.kind() == "optional_parameter" || default.is_some();
        if rest
            && (optional
                || (lang == Language::TypeScript && !matches!(ty_name, TypeContract::Array(_))))
        {
            return unsupported("Rest exige array TS explícito e não admite default/optional");
        }
        if contract
            .parameters
            .last()
            .is_some_and(|p| p.rest || (p.optional && !optional && !rest))
        {
            return unsupported("Ordem de parâmetros optional/rest inválida");
        }
        if let Some(value) = default
            && ((interface && lang == Language::TypeScript)
                || !literal_default(value, source, budget)?)
        {
            return unsupported(
                "Default exige literal constante sem execução ou resolução de nomes",
            );
        }
        let mut c = parameter.walk();
        for node in parameter.children(&mut c) {
            budget.node()?;
            if Some(node) != pattern
                && node != ty
                && Some(node) != default
                && !matches!(node.kind(), "comment" | "=" | "?" | "...")
            {
                return unsupported(
                    "Default, referência, decorator ou modificador de parâmetro não verificado",
                );
            }
        }
        contract.parameters.push(ParameterContract {
            name: source[name.byte_range()].into(),
            ty: ty_name,
            optional,
            rest,
            default: default.map(|n| source[n.byte_range()].into()),
        });
    }
    // A generator changes the callable contract even when its declared return is invalid.
    // Stay conservative also for yields nested inside a method's local function.
    let mut cursor = method.walk();
    loop {
        budget.node()?;
        if cursor.node().kind() == "yield_expression" {
            return unsupported("Generator exige verificação específica");
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(Ok(contract));
            }
        }
    }
}

// PHP nominal variance only; callers supply the already resolved, bounded
// inheritance graph. This does not infer structural or external type relations.
pub(crate) fn compatible_override(
    child: &MethodContract,
    base: &MethodContract,
    subtype: &mut impl FnMut(&str, &str) -> Result<bool, IssueKind>,
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    fn compatible(
        a: &TypeContract,
        b: &TypeContract,
        subtype: &mut impl FnMut(&str, &str) -> Result<bool, IssueKind>,
        budget: &mut Budget<'_>,
    ) -> Result<bool, IssueKind> {
        budget.node()?;
        match (a, b) {
            (TypeContract::Nominal(a), TypeContract::Nominal(b)) => subtype(a, b),
            (TypeContract::Union(a), b) => {
                for ty in a {
                    if !compatible(ty, b, subtype, budget)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (a, TypeContract::Union(b)) => {
                for ty in b {
                    if compatible(a, ty, subtype, budget)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // Mutable arrays stay invariant; no structural assignment is inferred.
            _ => Ok(a == b),
        }
    }
    if child.visibility != base.visibility
        || child.static_member != base.static_member
        || child.generics != base.generics
        || child.parameters.len() != base.parameters.len()
    {
        return Ok(false);
    }
    for (a, b) in child.parameters.iter().zip(&base.parameters) {
        budget.node()?;
        if a.name != b.name
            || a.optional != b.optional
            || a.rest != b.rest
            || a.default != b.default
            || !compatible(&b.ty, &a.ty, subtype, budget)?
        {
            return Ok(false);
        }
    }
    compatible(&child.returns, &base.returns, subtype, budget)
}
