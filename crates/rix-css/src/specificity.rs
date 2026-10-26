
use rix_dom::query::{ComplexSelector, CompoundSelector, PseudoClass, SimpleSelector};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Specificity {
    pub inline: u32,
    pub id: u32,
    pub class_attr_pseudo: u32,
    pub tag: u32,
}

impl Specificity {
    pub const INLINE: Self = Self {
        inline: 1,
        id: 0,
        class_attr_pseudo: 0,
        tag: 0,
    };

    pub const USER_AGENT: Self = Self {
        inline: 0,
        id: 0,
        class_attr_pseudo: 0,
        tag: 0,
    };

    pub const ZERO: Self = Self {
        inline: 0,
        id: 0,
        class_attr_pseudo: 0,
        tag: 0,
    };

    pub fn of_complex(selector: &ComplexSelector) -> Self {
        let mut spec = Self::default();
        for compound in &selector.chain {
            spec = spec.saturating_add(Self::of_compound(compound));
        }
        if selector.pseudo_element.is_some() {
            spec.tag = spec.tag.saturating_add(1);
        }
        spec
    }

    pub fn of_selector_list(selectors: &[ComplexSelector]) -> Self {
        selectors
            .iter()
            .map(Self::of_complex)
            .max()
            .unwrap_or(Self::ZERO)
    }

    pub fn of_selector_str(selector_text: &str) -> Self {
        match ComplexSelector::parse(selector_text) {
            Ok(list) => Self::of_selector_list(&list),
            Err(_) => Self::ZERO,
        }
    }

    pub fn of_compound(compound: &CompoundSelector) -> Self {
        let mut spec = Self::default();
        for pred in &compound.predicates {
            match pred {
                SimpleSelector::Universal => {}
                SimpleSelector::Tag(_) => spec.tag += 1,
                SimpleSelector::Id(_) => spec.id += 1,
                SimpleSelector::Class(_)
                | SimpleSelector::AttributeExact(..)
                | SimpleSelector::AttributePrefix(..)
                | SimpleSelector::AttributeSuffix(..)
                | SimpleSelector::AttributeContains(..)
                | SimpleSelector::AttributeWord(..)
                | SimpleSelector::AttributeDashMatch(..)
                | SimpleSelector::AttributeExists(..) => {
                    spec.class_attr_pseudo += 1;
                }
                SimpleSelector::Pseudo(p) => match p {
                    PseudoClass::Not(inner) => {
                        let sub_compound = CompoundSelector {
                            predicates: vec![*inner.clone()],
                            combinator_to_next: None,
                        };
                        spec = spec.saturating_add(Self::of_compound(&sub_compound));
                    }
                    PseudoClass::Is(inner) => {
                        spec = spec.saturating_add(Self::of_selector_list(inner));
                    }
                    PseudoClass::Where(_) => {}
                    PseudoClass::Host(inner) => {
                        spec.class_attr_pseudo += 1;
                        if let Some(compound) = inner {
                            spec = spec.saturating_add(Self::of_compound(compound));
                        }
                    }
                    PseudoClass::Slotted(inner) => {
                        spec.tag += 1;
                        spec = spec.saturating_add(Self::of_compound(inner));
                    }
                    _ => spec.class_attr_pseudo += 1,
                },
            }
        }
        spec
    }

    pub fn saturating_add(self, rhs: Self) -> Self {
        Self {
            inline: self.inline.saturating_add(rhs.inline),
            id: self.id.saturating_add(rhs.id),
            class_attr_pseudo: self.class_attr_pseudo.saturating_add(rhs.class_attr_pseudo),
            tag: self.tag.saturating_add(rhs.tag),
        }
    }

    pub fn to_u32(self) -> u32 {
        ((self.inline.min(255)) << 24)
            | ((self.id.min(255)) << 16)
            | ((self.class_attr_pseudo.min(255)) << 8)
            | (self.tag.min(255))
    }

    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

impl std::ops::Add for Specificity {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        self.saturating_add(rhs)
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.inline != other.inline {
            return self.inline.cmp(&other.inline);
        }
        if self.id != other.id {
            return self.id.cmp(&other.id);
        }
        if self.class_attr_pseudo != other.class_attr_pseudo {
            return self.class_attr_pseudo.cmp(&other.class_attr_pseudo);
        }
        self.tag.cmp(&other.tag)
    }
}
