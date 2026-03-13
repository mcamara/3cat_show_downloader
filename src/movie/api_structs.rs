//! Serde deserialization structs for the 3cat movie catalog page.
//!
//! Only the fields needed for movie ID lookup are deserialized.
//! Unknown fields are silently ignored by serde's default behavior.

use serde::Deserialize;

/// Root of the `__NEXT_DATA__` JSON embedded in the catalog page.
#[derive(Debug, Deserialize)]
pub struct CatalogRoot {
    /// Top-level props object.
    pub props: CatalogProps,
}

/// Contains the page-specific props.
#[derive(Debug, Deserialize)]
pub struct CatalogProps {
    /// Page props holding the layout definition.
    #[serde(rename = "pageProps")]
    pub page_props: PageProps,
}

/// Wraps the layout that contains the catalog structure.
#[derive(Debug, Deserialize)]
pub struct PageProps {
    /// The layout definition with its structure entries.
    pub layout: Layout,
}

/// The page layout, composed of a list of structure entries.
#[derive(Debug, Deserialize)]
pub struct Layout {
    /// Ordered list of structural components on the page.
    pub structure: Vec<StructureEntry>,
}

/// A single structural component; may optionally contain child entries.
#[derive(Debug, Deserialize)]
pub struct StructureEntry {
    /// Child components nested under this entry. Empty when absent.
    #[serde(default)]
    pub children: Vec<ChildEntry>,
}

/// A child entry within a structure component.
#[derive(Debug, Deserialize)]
pub struct ChildEntry {
    /// The resolved props for this child, which may contain movie items.
    #[serde(rename = "finalProps")]
    pub final_props: FinalProps,
}

/// Final resolved props that may carry a list of movie items.
#[derive(Debug, Deserialize)]
pub struct FinalProps {
    /// Movie items listed under this component. Empty when absent.
    #[serde(default)]
    pub items: Vec<MovieItem>,
}

/// A single movie entry from the catalog.
#[derive(Debug, Deserialize)]
pub struct MovieItem {
    /// Internal 3cat movie ID.
    pub id: i32,
    /// URL-friendly slug (e.g. `"iron-man"`).
    pub nom_friendly: String,
}
