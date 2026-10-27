//! The two wire surfaces, input and output, kept apart by type.

use super::ids::stringify_id_properties;
use super::inline::inline_ref_only_properties;
use super::scalarize::scalarize_slot_types;
use super::walks;
use super::WIRE_ID_KEYS;
use serde_json::{Map, Value};

// --- Les deux surfaces du fil, separees par le TYPE -------------------------
//
// Entree et sortie partageaient jusqu'ici la meme representation (un
// `Map<String, Value>` nu), donc rien n'empechait d'appliquer a une sortie
// une passe reservee a l'entree. Ce n'est pas une nuance de style :
// [`scalarize_slot_types`] effondre `anyOf: [T, null]` en `T`, et les SDK MCP
// valident `structuredContent` contre l'`outputSchema` annonce (spec
// 2025-06-18) — retirer la branche `null` d'une sortie ferait donc echouer,
// chez le client, les reponses legitimes du serveur lui-meme.
//
// Les deux surfaces sont donc deux types distincts, sans conversion de l'un
// vers l'autre.
//
// Portee exacte de la barriere, parce qu'une garantie surestimee est pire
// qu'une garantie absente. Elle tient sur ce qui SORT de ce module : hors
// d'ici, aucune fonction n'accepte une carte nue pour lui appliquer un
// durcissement d'ENTREE — les deux constructeurs de `WireInputSchema` sont
// prives au module, et le seul point de passage qui reprend un schema deja
// publie ([`reharden_tool_input`]) prend l'outil entier et ne touche que son
// `input_schema` : il n'y a pas de parametre par lequel passer une sortie.
// (Il a existe, sous la forme d'un `pub(crate) fn(&JsonObject)`, et
// `rmcp::model::Tool` type ses deux schemas avec le MEME
// `Arc<JsonObject>` : rien n'aurait signale l'erreur.)
//
// Elle ne tient PAS a l'interieur de `schema.rs` : `scalarize_in_map` est
// typee sur la carte brute et `WireOutputSchema::harden` vit dans le meme
// module. Ce que le type empeche, c'est l'accident a distance ; ici, seule
// la lecture protege — d'ou le garde executable
// `output_hardening_keeps_a_nullable_union_intact` dans `schema_tests.rs`,
// qui echoue si la scalarisation atteint un jour la sortie.

/// Un schema d'ENTREE en construction : ce que le client LIT pour fabriquer
/// ses arguments. Il doit survivre au harnais qui le rend, pas seulement etre
/// valide au sens JSON Schema.
pub(crate) struct WireInputSchema(Map<String, Value>);

/// Un schema de SORTIE en construction : ce que le SDK client valide contre
/// la reponse. Il doit rester fidele a ce que le serveur emet reellement,
/// unions nullables comprises.
pub(crate) struct WireOutputSchema(Map<String, Value>);

impl WireInputSchema {
    /// Le schema derive par `schemars` pour les parametres de l'outil, avant
    /// tout durcissement.
    fn derived<T: schemars::JsonSchema + std::any::Any>() -> Self {
        let schema = rmcp::handler::server::tool::schema_for_input::<
            rmcp::handler::server::wrapper::Parameters<T>,
        >()
        .unwrap_or_else(|e| {
            panic!(
                "Invalid input schema for {}: {e}",
                std::any::type_name::<T>()
            )
        });
        Self((*schema).clone())
    }

    /// Reprend un schema d'entree DEJA publie par une route, pour lui
    /// repasser le durcissement universel. Volontairement reserve a la famille
    /// `schema` (visible de ses modules freres et de ses tests, pas du crate) :
    /// c'est le seul constructeur qui accepte une carte venue de l'exterieur,
    /// et son unique appelant est [`reharden_tool_input`], dont le nom
    /// dit la surface.
    pub(super) fn adopt(map: Map<String, Value>) -> Self {
        Self(map)
    }

    /// La carte en cours de construction, pour les passes des modules freres
    /// qui reecrivent un schema d'ENTREE en place. Le type reste la barriere :
    /// seule une `WireInputSchema` donne acces a cette carte.
    pub(super) fn map_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.0
    }

    /// La reecriture des liens rustdoc passe avant tout : elle ne touche que
    /// les `description`, donc rien de ce qui suit n'en depend. Ensuite,
    /// l'ordre compte. `stringify_id_properties` d'abord, pour que les copies
    /// faites par l'inliner heritent du type deja pose ; l'inliner ensuite,
    /// pour que chaque branche porte un `type` direct ; la scalarisation en
    /// dernier, parce qu'elle ne promeut qu'une branche deja typee.
    fn harden(mut self, id_keys: &[&str]) -> Self {
        walks::unlink_rustdoc_descriptions(&mut self.0);
        stringify_id_properties(&mut self.0, id_keys);
        inline_ref_only_properties(&mut self.0);
        scalarize_slot_types(&mut self);
        self
    }

    fn publish(self) -> std::sync::Arc<rmcp::model::JsonObject> {
        std::sync::Arc::new(self.0)
    }
}

impl WireOutputSchema {
    fn derived<T: schemars::JsonSchema + std::any::Any>() -> Self {
        let schema = rmcp::handler::server::tool::schema_for_output::<T>();
        Self((*schema).clone())
    }

    /// Liens rustdoc reecrits, puis elargissement des ids, puis inlining. Pas
    /// de scalarisation : voir le commentaire de section.
    fn harden(mut self) -> Self {
        walks::unlink_rustdoc_descriptions(&mut self.0);
        walks::widen_id_properties(&mut self.0, WIRE_ID_KEYS);
        inline_ref_only_properties(&mut self.0);
        self
    }

    fn publish(self) -> std::sync::Arc<rmcp::model::JsonObject> {
        std::sync::Arc::new(self.0)
    }
}

/// Le schema d'ENTREE annonce d'un outil.
///
/// `id_keys` nomme les proprietes que CET outil accepte en chaine decimale
/// (cf. [`stringify_id_properties`]) — la tolerance est per-outil, pas
/// globale : `explain_compilation.fragment_id` est un `u64` STRICT et ne doit
/// jamais etre annonce `string`.
///
/// Constructeur unique des deux qui existaient : celui de `mcp.rs` (jeu de
/// cles par outil) et celui de `mcp/context_tools.rs` (cle `"id"` figee).
pub(crate) fn wire_safe_input_schema<T: schemars::JsonSchema + std::any::Any>(
    id_keys: &[&str],
) -> std::sync::Arc<rmcp::model::JsonObject> {
    WireInputSchema::derived::<T>().harden(id_keys).publish()
}

/// Repasse le durcissement universel (liens rustdoc, inlining + scalarisation) sur le
/// schema d'ENTREE d'un outil deja construit.
///
/// C'est le point de passage unique de `McpServer::combined_router` : une
/// route qui n'a declare aucun `input_schema` recoit celui derive par rmcp,
/// que rien ne post-traitait. Sans cles d'id — la tolerance d'un id est une
/// connaissance de l'outil, elle reste dans son attribut.
///
/// Prend l'outil ENTIER, et non son `input_schema` : `rmcp::model::Tool`
/// type `input_schema` et `output_schema` avec le meme `Arc<JsonObject>`,
/// donc une signature `fn(&JsonObject)` aurait accepte un schema de sortie
/// sans le moindre diagnostic — et scalariser une sortie fait echouer, chez
/// le client, la validation des reponses legitimes du serveur. Ici, la
/// sortie n'est pas atteignable : elle n'est pas un parametre.
pub(crate) fn reharden_tool_input(tool: &mut rmcp::model::Tool) {
    tool.input_schema = WireInputSchema::adopt((*tool.input_schema).clone())
        .harden(&[])
        .publish();
}

/// Le schema de sortie ANNONCE d'un outil : liens rustdoc reecrits, ids
/// elargis, puis `$ref` inlines et `$defs` inatteignables elagues.
///
/// Vit ici plutot que dans `mcp/context_tools.rs` parce que les outils de
/// `mcp.rs` l'appellent aussi : le laisser dans un module gate sur `context`
/// cassait `--features http`, qui active `mcp` sans `context`.
pub(crate) fn wire_safe_output_schema<T: schemars::JsonSchema + std::any::Any>(
) -> std::sync::Arc<rmcp::model::JsonObject> {
    WireOutputSchema::derived::<T>().harden().publish()
}
