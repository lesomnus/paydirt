//! Build typed decorators from the server traits emitted by the pinned Connect generator.
//! Reuse their signatures, including zero-copy views and stream capture bounds.
use crate::{Result, generate::write_file};
use quote::{format_ident, quote};
use std::{fs, io::Write, path::Path};
use syn::{
    CapturedParam, File, FnArg, GenericArgument, Item, ItemTrait, Pat, PathArguments, TraitItem,
    Type, TypeParamBound, parse_quote,
    punctuated::Punctuated,
    visit_mut::{self, VisitMut},
};

struct Captures<'a>(&'a [CapturedParam]);

impl VisitMut for Captures<'_> {
    fn visit_precise_capture_mut(&mut self, capture: &mut syn::PreciseCapture) {
        let mut args = Punctuated::new();
        for param in &capture.params {
            if matches!(param, CapturedParam::Ident(name) if name == "Self") {
                args.extend(self.0.iter().cloned());
            } else {
                args.push(param.clone());
            }
        }
        capture.params = args;
    }
}

// The bottom of an incomplete stack returns an error. Name an owned response
// type so that Rust can infer its Encodable even when no body is produced.
struct OwnedResponse;
impl VisitMut for OwnedResponse {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::ImplTrait(t) = ty {
            for bound in &t.bounds {
                if let TypeParamBound::Trait(b) = bound
                    && let Some(seg) = b.path.segments.last()
                    && seg.ident == "Encodable"
                    && let PathArguments::AngleBracketed(a) = &seg.arguments
                    && let Some(GenericArgument::Type(message)) = a.args.first()
                {
                    *ty = message.clone();
                    return;
                }
            }
        }
        visit_mut::visit_type_mut(self, ty);
    }
}

pub(crate) fn concrete_signature(mut sig: syn::Signature) -> syn::Signature {
    Captures(&[]).visit_signature_mut(&mut sig);
    sig
}

fn layer(service: &ItemTrait) -> Result<File> {
    let name = &service.ident;
    let hooks = format_ident!("{name}Layer");
    let overlay = format_ident!("{name}Overlay");
    let bottom = format_ident!("{name}Unimplemented");
    let mut defaults = Vec::new();
    let mut forwards = Vec::new();
    let mut missing = Vec::new();
    for item in &service.items {
        let TraitItem::Fn(method) = item else {
            return Err(format!("unexpected item in Connect trait {name}").into());
        };
        let sig = &method.sig;
        let rpc = &sig.ident;
        let receiver = sig.receiver().ok_or("missing Connect service receiver")?;
        let lifetime = &receiver
            .reference
            .as_ref()
            .ok_or("expected borrowed service")?
            .1;
        let args: Vec<_> = sig
            .inputs
            .iter()
            .skip(1)
            .map(|arg| match arg {
                FnArg::Typed(arg) => match &*arg.pat {
                    Pat::Ident(pat) => Ok(&pat.ident),
                    _ => Err("expected named Connect argument"),
                },
                _ => Err("unexpected receiver"),
            })
            .collect::<std::result::Result<_, _>>()?;
        let mut hook = sig.clone();
        hook.inputs.insert(1, parse_quote!(next: & #lifetime N));
        Captures(&[parse_quote!(Self), parse_quote!(N)]).visit_signature_mut(&mut hook);
        defaults.push(quote!(#hook { next.#rpc(#(#args),*) }));

        let mut forward = sig.clone();
        Captures(&[parse_quote!(N), parse_quote!(L)]).visit_signature_mut(&mut forward);
        forwards.push(quote!(#forward { self.layer.#rpc(&self.next, #(#args),*) }));

        let mut fallback = sig.clone();
        OwnedResponse.visit_signature_mut(&mut fallback);
        missing.push(quote!(#fallback {
            let _ = (#(#args),*);
            async { Err(::connectrpc::ConnectError::unimplemented(concat!(stringify!(#name), ".", stringify!(#rpc)))) }
        }));
    }
    Ok(parse_quote! {
        pub trait #hooks<N: #name>: Send + Sync + 'static { #(#defaults)* }
        pub struct #overlay<N, L> { pub next: N, pub layer: L }
        impl<N: #name, L: #hooks<N>> #name for #overlay<N, L> { #(#forwards)* }
        pub struct #bottom;
        #[allow(refining_impl_trait)]
        impl #name for #bottom { #(#missing)* }
    })
}

pub fn generate(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(stem) = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_suffix(".__connect.rs"))
        else {
            continue;
        };
        let syntax = syn::parse_file(&fs::read_to_string(&path)?)?;
        let mut items = Vec::new();
        for item in &syntax.items {
            if let Item::Trait(t) = item
                && t.supertraits
                    .iter()
                    .any(|b| matches!(b, TypeParamBound::Trait(b) if b.path.is_ident("Send")))
            {
                items.extend(layer(t)?.items);
            }
        }
        if items.is_empty() {
            continue;
        }
        let file_name = format!("{stem}.__layers.rs");
        let file = File {
            shebang: None,
            attrs: Vec::new(),
            items,
        };
        write_file(
            &dir.join(&file_name),
            format!(
                "// @generated by paydirt.\n{}",
                prettyplease::unparse(&file)
            )
            .as_bytes(),
        )?;
        writeln!(
            fs::OpenOptions::new().append(true).open(path)?,
            "include!({file_name:?});"
        )?;
    }
    Ok(())
}
