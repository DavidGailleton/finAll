//! Reusable UI components shared across pages. These run on both targets (SSR
//! and hydration) and hold no financial or DB logic — display formatting only.

mod auth_card;
mod button;
mod form_error;
mod layout;
mod money;
mod page_header;
mod panel;
mod scrollable_table;
mod select_field;
mod text_field;

pub use auth_card::AuthCard;
pub use button::Button;
pub use form_error::FormError;
pub use layout::Layout;
pub use money::Money;
pub use page_header::PageHeader;
pub use panel::Panel;
pub use scrollable_table::ScrollableTable;
pub use select_field::SelectField;
pub use text_field::TextField;
