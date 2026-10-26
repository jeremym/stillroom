//! Stillroom core: recipe types, validation against the JSON Schema, and (later) the executor.

pub mod recipe;

pub use recipe::{Recipe, RecipeError, RECIPE_SCHEMA};
