use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::spanned::Spanned;
use syn::token::Colon;
use syn::{
    Block, Expr, ExprLit, Ident, ItemFn, Lit, LitStr, Macro, Result, Token,
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote,
    token::{Brace, Not},
};
use syn::{FnArg, PatType, Signature, Stmt, Type, TypeReference};

/// A procedural macro that provides JSX-like syntax for creating HTML elements in Rust.
///
/// # Examples
///
/// ```rust
/// use simple_rsx::*;
/// // Fragment
/// rsx!(<>"Hello World"</>);
///
/// // Self-closing tag
/// rsx!(<div class="container" id="app" />);
///
/// // Tag with children
/// rsx!(<div class="container">
///     <h1>Title</h1>
///     <p>Paragraph text</p>
/// </div>);
///
/// // Expression
/// let name = "World";
/// rsx!(<div>Hello {name}</div>);
/// ```
#[proc_macro]
pub fn rsx(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as RsxNode);
    let expanded = input.to_tokens();
    expanded.into()
}
/// A procedural macro that transforms a conditional expression into a JSX-like syntax.
///
/// # Examples
/// ```rust
/// use simple_rsx::*;
/// // Fragment
/// let show = true;
/// either!(show => <p>"Show me"</p>);
/// ```
#[proc_macro]
pub fn either(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as Either);
    let expanded = input.to_tokens();
    expanded.into()
}

struct Either {
    condition: Expr,
    true_value: RsxNode,
    false_value: Option<RsxNode>,
}

impl Parse for Either {
    fn parse(input: ParseStream) -> Result<Self> {
        let condition = input.parse()?;
        input.parse::<Token![=>]>()?;
        let true_value = input.parse()?;
        let false_value = if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Either {
            condition,
            true_value,
            false_value,
        })
    }
}

impl Either {
    fn to_tokens(&self) -> TokenStream2 {
        let condition = &self.condition;
        let false_value = &self
            .false_value
            .as_ref()
            .map(|v| v.to_tokens())
            .or_else(|| Some(quote! {::simple_rsx::Node::Fragment(vec![])}));
        let true_value = self.true_value.to_tokens();

        quote! {
            if #condition {
                #true_value.into()
            } else {
                #false_value
            }
        }
    }
}

/// A procedural macro that transforms a rust function into a component.
///
/// # Examples
///
/// ```rust
/// use simple_rsx::*;
///
/// #[component]
/// fn HelloWorld() -> Node {
///     rsx!(<div>Hello World</div>)
/// }
/// ```
#[proc_macro_attribute]
pub fn component(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let ItemFn {
        vis,
        attrs,
        sig,
        block,
    } = parse_macro_input!(input as ItemFn);
    let Signature {
        ident,
        asyncness,
        constness,
        unsafety,
        mut inputs,
        output,
        fn_token,
        ..
    } = sig;

    if asyncness.is_some() || constness.is_some() || unsafety.is_some() {
        panic!("async, const, and unsafe functions are not supported");
    }

    if inputs.len() > 1 {
        panic!("Components can only take a single prop as input");
    }

    let prop_ty = inputs
        .iter()
        .map(|input| match input {
            FnArg::Typed(PatType { ty, .. }) => match &**ty {
                Type::Reference(TypeReference { elem, .. }) => elem,
                _ => ty,
            },
            _ => panic!("Only typed inputs are supported"),
        })
        .next();

    let prop_type = if let Some(prop_ty) = prop_ty {
        quote! {type Props = #prop_ty;}
    } else {
        quote! {type Props = ::simple_rsx::PropWithChildren;}
    };

    if inputs.is_empty() {
        inputs.push(FnArg::Typed(PatType {
            attrs: Vec::new(),
            pat: parse_quote!(_),
            colon_token: Colon::default(),
            ty: parse_quote!(&Self::Props),
        }));
    }

    let expanded = quote! {
        #vis #(#attrs)* struct #ident;

        impl ::simple_rsx::Component for #ident {
            #prop_type
            #fn_token render(#inputs) #output #block
        }
    };

    expanded.into()
}

/// Represents the different types of JSX nodes
#[derive(Debug)]
enum RsxNode {
    Fragment(Vec<RsxNode>),
    Component {
        name: Ident,
        props: Vec<(Ident, Option<Block>)>,
        children: Vec<RsxNode>,
        close_tag: Option<Ident>,
    },
    Text(Expr),
    Block(Block),
    Empty,
    Comment(Expr), // HTML comments
}

struct NodeBlock {
    expr: Option<Expr>,
    value: Option<Block>,
}

impl Parse for NodeBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(LitStr) {
            let parsed: LitStr = input.parse()?;
            return Ok(NodeBlock {
                value: None,
                expr: Some(syn::Expr::Macro(syn::ExprMacro {
                    attrs: Vec::new(),
                    mac: Macro {
                        path: parse_quote!(format),
                        bang_token: Not::default(),
                        delimiter: syn::MacroDelimiter::Paren(syn::token::Paren::default()),
                        tokens: quote::quote!(#parsed),
                    },
                })),
            });
        }

        let is_block = input.to_string().trim().starts_with('{');

        if is_block {
            let value: Block = input.parse()?;
            return Ok(NodeBlock {
                value: Some(value),
                expr: None,
            });
        }

        if input.lookahead1().peek(Token![<]) {
            // Found a non-literal '<', stop here without consuming it
            return Ok(NodeBlock {
                value: None,
                expr: None,
            });
        }

        match input.parse::<proc_macro2::TokenTree>() {
            Ok(token) => match &token {
                proc_macro2::TokenTree::Group(group) => {
                    let stream = group.stream();
                    let expr = syn::parse2::<Expr>(stream)?;
                    Ok(NodeBlock {
                        value: None,
                        expr: Some(expr),
                    })
                }
                _ => {
                    let value = token.to_string();
                    let str_expr = syn::Expr::Lit(ExprLit {
                        attrs: Vec::new(),
                        lit: Lit::Str(LitStr::new(&value, token.span())),
                    });
                    Ok(NodeBlock {
                        value: None,
                        expr: Some(str_expr),
                    })
                }
            },
            Err(e) => Err(e), // End of input
        }
    }
}

/// Represents an attribute name-value pair
struct NodeValue {
    name: Ident,
    value: Option<Block>,
}

impl Parse for NodeValue {
    fn parse(input: ParseStream) -> Result<Self> {
        let name = input.parse()?;
        if !input.peek(Token![=]) {
            return Ok(NodeValue { name, value: None });
        }
        input.parse::<Token![=]>()?;
        let NodeBlock { value, expr } = input.parse()?;
        Ok(NodeValue {
            name,
            value: value.or_else(|| {
                expr.map(|expr| Block {
                    brace_token: Brace::default(),
                    stmts: vec![syn::Stmt::Expr(expr, None)],
                })
            }),
        })
    }
}

struct RsxChildren {
    children: Vec<RsxNode>,
}

impl Parse for RsxChildren {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut children = Vec::with_capacity(4);
        let mut last_end = 0;
        while !(input.is_empty() || input.peek(Token![<]) && input.peek2(Token![/])) {
            let span_info = format!("{:?}", input.span());
            let (start, end) = parse_range(&span_info).unwrap_or((0, 0));
            match input.parse::<RsxNode>() {
                Ok(child) => children.push(child),
                Err(_) => {
                    let mut value = String::new();
                    let token = input.parse::<proc_macro2::TokenTree>()?;

                    if !matches!(token, proc_macro2::TokenTree::Punct(_)) {
                        let gap_size = start - last_end;
                        if gap_size > 0 && last_end > 0 {
                            // Add spaces to represent the gap
                            value.push_str(&" ".repeat(gap_size as usize));
                        }
                    }
                    value.push_str(&token.to_string());

                    children.push(RsxNode::Text(syn::Expr::Lit(ExprLit {
                        attrs: Vec::new(),
                        lit: Lit::Str(LitStr::new(&value, token.span())),
                    })));
                }
            }
            last_end = end;
        }

        Ok(RsxChildren { children })
    }
}

impl Parse for RsxNode {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(RsxNode::Empty);
        }

        // Look ahead to see if we start with a '<'
        if input.peek(Token![<]) {
            input.parse::<Token![<]>()?;

            // Comments: <!-- ... -->
            if input.peek(Token![!]) && input.peek2(Token![-]) && input.peek3(Token![-]) {
                input.parse::<Token![!]>()?;
                input.parse::<Token![-]>()?;
                input.parse::<Token![-]>()?;

                let mut nodes = Vec::new();
                while !(input.is_empty()
                    || input.peek(Token![-]) && input.peek2(Token![-]) && input.peek3(Token![>]))
                {
                    let mut comment = String::new();
                    let token = input.parse::<proc_macro2::TokenTree>()?;
                    comment.push(' ');
                    comment.push_str(&token.to_string());
                    nodes.push(LitStr::new(&comment, token.span()));
                }

                let token = input.parse::<Token![-]>()?;
                nodes.push(LitStr::new(" ", token.span()));
                input.parse::<Token![-]>()?;
                input.parse::<Token![>]>()?;

                // concat all nodes into a single lit
                // Convert each LitStr to an Expr::Lit
                let exprs: Vec<Expr> = nodes
                    .into_iter()
                    .map(|lit| {
                        Expr::Lit(syn::ExprLit {
                            attrs: vec![],
                            lit: syn::Lit::Str(lit),
                        })
                    })
                    .collect();

                // Build a binary expression tree with the + operator
                let mut result = syn::parse_str::<Expr>("String::new()").unwrap();

                for expr in exprs.into_iter() {
                    result = Expr::Binary(syn::ExprBinary {
                        attrs: vec![],
                        left: Box::new(result),
                        op: syn::BinOp::Add(syn::token::Plus::default()),
                        right: Box::new(expr),
                    });
                }

                return Ok(RsxNode::Comment(result));
            }

            // Fragment: <>...</>
            if input.peek(Token![>]) {
                input.parse::<Token![>]>()?;

                let RsxChildren { children } = input.parse()?;

                input.parse::<Token![<]>()?;
                input.parse::<Token![/]>()?;
                input.parse::<Token![>]>()?;

                return Ok(RsxNode::Fragment(children));
            }

            // Element: <tag ...>...</tag> or <tag ... />
            let tag = input.parse::<Ident>()?;

            let mut attributes = Vec::with_capacity(4);
            while !input.peek(Token![>]) && !input.peek(Token![/]) {
                if input.to_string().trim().starts_with('{') {
                    let expr = input.parse::<Block>()?;
                    // check if expr matches {Ident} pattern
                    if let Some(Stmt::Expr(expr, token)) = expr.stmts.first() {
                        if let Expr::Path(expr_path) = expr {
                            match expr_path.path.segments.first() {
                                Some(segment) => {
                                    let ident = segment.ident.clone();
                                    attributes.push((
                                        ident,
                                        Some(Block {
                                            brace_token: Brace::default(),
                                            stmts: vec![syn::Stmt::Expr(expr.clone(), *token)],
                                        }),
                                    ));
                                }
                                _ => {
                                    return Err(syn::Error::new(
                                        expr_path.span(),
                                        "Only Ident expressions are supported",
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    match input.parse::<NodeValue>() {
                        Ok(attr) => attributes.push((attr.name, attr.value)),
                        Err(e) => return Err(e),
                    }
                }
            }

            // Self-closing tag: <tag ... /> or <Component... />
            if input.peek(Token![/]) {
                input.parse::<Token![/]>()?;
                input.parse::<Token![>]>()?;

                return Ok(RsxNode::Component {
                    name: tag.clone(),
                    props: attributes,
                    children: Vec::new(),
                    close_tag: None,
                });
            }

            // Opening tag ends: <tag ...>
            input.parse::<Token![>]>()?;

            let RsxChildren { children } = input.parse()?;

            // Closing tag: </tag>
            input.parse::<Token![<]>()?;
            input.parse::<Token![/]>()?;
            let close_tag = input.parse::<Ident>()?;

            // Validate matching tags
            if tag != close_tag {
                return Err(syn::Error::new(
                    close_tag.span(),
                    format!(
                        "Closing tag </{}> doesn't match opening tag <{}>",
                        close_tag, tag
                    ),
                ));
            }

            input.parse::<Token![>]>()?;

            return Ok(RsxNode::Component {
                name: tag,
                props: attributes,
                children,
                close_tag: Some(close_tag),
            });
        }

        // Text content or expression
        if input.peek(Lit) {
            let lit: Lit = input.parse()?;
            let expr = Expr::Lit(ExprLit {
                attrs: Vec::new(),
                lit,
            });
            return Ok(RsxNode::Text(expr));
        }
        match input.parse::<Block>() {
            Ok(block) => Ok(RsxNode::Block(block)),
            Err(_) => Err(syn::Error::new(
                Span::call_site(),
                "Invalid JSX node, expected a valid rsx block, an expression or plain text",
            )),
        }
    }
}

impl RsxNode {
    fn to_tokens(&self) -> TokenStream2 {
        match self {
            RsxNode::Component {
                name,
                props,
                children,
                close_tag,
            } => {
                let is_element = name.to_string().starts_with(|c: char| !c.is_uppercase());

                let attrs = props
                    .iter() // filter out data- attributes for elements
                    .map(|(name, value)| {
                        let value = value
                            .as_ref()
                            .map(|v| quote! {#v})
                            .or_else(|| Some(quote! {true}));
                        (name, value)
                    });

                let data_props = (is_element
                    && props
                        .iter()
                        .any(|(name, _)| name.to_string().starts_with("data_")))
                .then(|| {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
                        .as_nanos()
                        .to_string();
                    let ident =
                        syn::Ident::new(&format!("attr_data_{}", timestamp), Span::call_site());
                    let data = attrs
                        .clone()
                        .filter(|(name, _)| name.to_string().starts_with("data_"))
                        .map(|(name, value)| {
                            quote! {
                                let #name = #value;
                                #ident.insert(stringify!(#name).to_string(), #name);
                            }
                        });
                    quote! {
                        r#data: {
                            let mut #ident = std::collections::HashMap::new();
                            {
                                #(#data)*
                            }
                            #ident
                        },
                    }
                });
                let props_tokens = attrs
                    .filter(|(name, _)| !(is_element && name.to_string().starts_with("data_"))) // filter out data- attributes for elements
                    .map(|(name, value)| quote! { #name: {#value}.into(), });

                let child_tokens = children.iter().map(|child| child.to_tokens());
                let children_tokens = quote! {
                    children: vec![#(#child_tokens),*],
                };

                let use_element = is_element.then(|| quote! {use ::simple_rsx::elements::#name;});
                let close_tag = close_tag.as_ref().map(|close_tag| {
                    quote! {
                        {
                            #use_element
                            let #close_tag = #name;
                        };
                    }
                });
                let default_props = is_element.then(|| quote! {..Default::default()});

                let component = if !is_element {
                    quote! { #name }
                } else {
                    quote! { ::simple_rsx::elements::#name }
                };

                quote! {
                    {
                        type Props = <#component as ::simple_rsx::Component>::Props;
                        {
                            #close_tag
                            ::simple_rsx::dom::render_component::<#component>(
                                Props {
                                    #(#props_tokens)*
                                    #children_tokens
                                    #data_props
                                    #default_props
                                },
                                |_| {},
                            )
                        }.unwrap()
                    }
                }
            }
            RsxNode::Fragment(children) => {
                let children_tokens = children.iter().map(|child| child.to_tokens());

                quote! {
                    {
                        ::simple_rsx::Node::Fragment(vec![#(#children_tokens),*])
                    }
                }
            }
            RsxNode::Text(expr) => {
                quote! {
                    {
                        ::simple_rsx::Node::from(#expr)
                    }
                }
            }
            RsxNode::Empty => {
                quote! {
                    ::simple_rsx::Node::Empty
                }
            }
            RsxNode::Comment(expr) => {
                quote! {
                    ::simple_rsx::Node::Comment(#expr)
                }
            }
            RsxNode::Block(block) => {
                quote! {
                    ::simple_rsx::Node::from(#block)
                }
            }
        }
    }
}

fn parse_range(input: &str) -> Option<(usize, usize)> {
    use regex::Regex;
    let re = Regex::new(r"(\d+)\.\.(\d+)").ok()?;
    let captures = re.captures(input)?;
    let start = captures.get(1)?.as_str().parse::<usize>().ok()?;
    let end = captures.get(2)?.as_str().parse::<usize>().ok()?;

    Some((start, end))
}
