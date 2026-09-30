//! The class tree of a [`ClassificationDefinition`], checked, and the
//! property names that read a classification in the reserved set
//! [`crate::CLASSIFICATION_SET`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::ClassificationDefinition;

/// The checked classes of one classification, with each class's parent and
/// level.
///
/// A hierarchical classification's tree holds its declared classes; a flat
/// one's holds the class names its rows assign, each a root at level 1 with
/// no code. Lookups are by class id and answer in a stable order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassTree {
    hierarchical: bool,
    classes: BTreeMap<String, ClassNode>,
    depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClassNode {
    parent: Option<String>,
    level: usize,
    code: Option<String>,
}

impl ClassTree {
    /// The tree of `definition`, refusing blank or duplicate ids and codes,
    /// an undeclared or cyclic parent, and a row naming an undeclared class.
    /// The error describes the first problem found, in declaration order.
    pub fn of(definition: &ClassificationDefinition) -> Result<Self, String> {
        if definition.classes.is_empty() {
            let classes = definition
                .rows
                .iter()
                .map(|row| {
                    let node = ClassNode {
                        parent: None,
                        level: 1,
                        code: None,
                    };
                    (row.class.clone(), node)
                })
                .collect::<BTreeMap<_, _>>();
            let depth = usize::from(!classes.is_empty());
            return Ok(Self {
                hierarchical: false,
                classes,
                depth,
            });
        }
        let mut parents: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        let mut codes = BTreeSet::new();
        for (index, class) in definition.classes.iter().enumerate() {
            if class.id.trim().is_empty() {
                return Err(format!("class {index} has a blank id"));
            }
            if parents
                .insert(class.id.as_str(), class.parent.as_deref())
                .is_some()
            {
                return Err(format!("the class `{}` is declared twice", class.id));
            }
            if let Some(code) = &class.code {
                if code.trim().is_empty() {
                    return Err(format!("the class `{}` has a blank code", class.id));
                }
                if !codes.insert(code.as_str()) {
                    return Err(format!("two classes have the code `{code}`"));
                }
            }
        }
        for class in &definition.classes {
            if let Some(parent) = &class.parent
                && !parents.contains_key(parent.as_str())
            {
                return Err(format!(
                    "the class `{}` names the undeclared parent `{parent}`",
                    class.id
                ));
            }
        }
        let mut classes = BTreeMap::new();
        let mut depth = 0;
        for class in &definition.classes {
            // Walking up from a class reaches a root within as many steps as
            // there are classes, or the parents form a cycle.
            let mut level = 1;
            let mut at = class.parent.as_deref();
            while let Some(parent) = at {
                if level > parents.len() {
                    return Err(format!(
                        "the parents of the class `{}` form a cycle",
                        class.id
                    ));
                }
                level += 1;
                at = parents[parent];
            }
            depth = depth.max(level);
            let node = ClassNode {
                parent: class.parent.clone(),
                level,
                code: class.code.clone(),
            };
            classes.insert(class.id.clone(), node);
        }
        for (index, row) in definition.rows.iter().enumerate() {
            if !classes.contains_key(&row.class) {
                return Err(format!(
                    "row {index} assigns the undeclared class `{}`",
                    row.class
                ));
            }
        }
        Ok(Self {
            hierarchical: true,
            classes,
            depth,
        })
    }

    /// Whether the classification declares its classes as a tree.
    #[must_use]
    pub const fn is_hierarchical(&self) -> bool {
        self.hierarchical
    }

    /// The deepest level of the tree; 1 for a flat classification with rows.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Whether `class` is one of the tree's classes.
    #[must_use]
    pub fn contains(&self, class: &str) -> bool {
        self.classes.contains_key(class)
    }

    /// The level of `class`, 1 for a root; `None` for an unknown class.
    #[must_use]
    pub fn level(&self, class: &str) -> Option<usize> {
        self.classes.get(class).map(|node| node.level)
    }

    /// The parent of `class`; `None` for a root or an unknown class.
    #[must_use]
    pub fn parent(&self, class: &str) -> Option<&str> {
        self.classes.get(class)?.parent.as_deref()
    }

    /// The declared code of `class`, if it has one.
    #[must_use]
    pub fn code(&self, class: &str) -> Option<&str> {
        self.classes.get(class)?.code.as_deref()
    }

    /// The class at `level` on the way from `class` to its root: `class`
    /// itself at its own level, an ancestor above it. `None` when `class`
    /// lies above `level` or is unknown.
    #[must_use]
    pub fn at_level<'a>(&'a self, class: &'a str, level: usize) -> Option<&'a str> {
        let mut at = class;
        let mut node = self.classes.get(at)?;
        if node.level < level {
            return None;
        }
        while node.level > level {
            at = node.parent.as_deref()?;
            node = self.classes.get(at)?;
        }
        Some(at)
    }

    /// Whether `class` is `ancestor` or lies below it.
    #[must_use]
    pub fn is_within(&self, class: &str, ancestor: &str) -> bool {
        self.level(ancestor)
            .is_some_and(|level| self.at_level(class, level) == Some(ancestor))
    }
}

/// A property name in the reserved set [`crate::CLASSIFICATION_SET`]: a
/// classification's id, reading the class it assigns, optionally followed
/// by `;level=<n>`, reading the class at level `n` of a hierarchical
/// classification's tree on the way from the assigned class to its root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassificationProperty<'a> {
    /// The classification's id.
    pub classification: &'a str,
    /// The tree level read, 1 for the roots; `None` reads the assigned class.
    pub level: Option<usize>,
}

impl<'a> ClassificationProperty<'a> {
    /// Parses `name`, refusing any parameter but one `level`, and a level
    /// that is not a positive integer in its canonical spelling.
    pub fn parse(name: &'a str) -> Result<Self, String> {
        let Some((classification, parameter)) = name.split_once(';') else {
            return Ok(Self {
                classification: name,
                level: None,
            });
        };
        let Some(level) = parameter.strip_prefix("level=") else {
            return Err(format!(
                "`{parameter}` is not a classification parameter; only `level=<n>` is"
            ));
        };
        match level.parse::<usize>() {
            Ok(value) if value > 0 && value.to_string() == level => Ok(Self {
                classification,
                level: Some(value),
            }),
            _ => Err(format!("the level `{level}` is not a positive integer")),
        }
    }
}

impl fmt::Display for ClassificationProperty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.classification)?;
        if let Some(level) = self.level {
            write!(f, ";level={level}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ClassDefinition, ClassificationMode, ClassificationRow, LocalizedText, Selector,
    };

    fn class(id: &str, code: Option<&str>, parent: Option<&str>) -> ClassDefinition {
        ClassDefinition {
            id: id.into(),
            code: code.map(Into::into),
            name: LocalizedText::plain(id),
            parent: parent.map(Into::into),
        }
    }

    fn definition(classes: Vec<ClassDefinition>, rows: &[&str]) -> ClassificationDefinition {
        ClassificationDefinition {
            id: "cost-group".into(),
            name: LocalizedText::plain("Cost group"),
            description: None,
            mode: ClassificationMode::FirstMatch,
            rows: rows
                .iter()
                .map(|class| ClassificationRow {
                    selector: Selector::All,
                    class: (*class).into(),
                })
                .collect(),
            classes,
        }
    }

    fn cost_groups() -> Vec<ClassDefinition> {
        vec![
            class("300", Some("300"), None),
            class("330", Some("330"), Some("300")),
            class("331", Some("331"), Some("330")),
            class("332", Some("332"), Some("330")),
            class("340", Some("340"), Some("300")),
        ]
    }

    #[test]
    fn levels_and_ancestors_follow_the_parents() {
        let tree = ClassTree::of(&definition(cost_groups(), &["331", "340"])).unwrap();
        assert!(tree.is_hierarchical());
        assert_eq!(tree.depth(), 3);
        assert_eq!(tree.level("331"), Some(3));
        assert_eq!(tree.parent("331"), Some("330"));
        assert_eq!(tree.code("332"), Some("332"));
        assert_eq!(tree.at_level("331", 1), Some("300"));
        assert_eq!(tree.at_level("331", 2), Some("330"));
        assert_eq!(tree.at_level("331", 3), Some("331"));
        assert_eq!(tree.at_level("340", 3), None);
        assert!(tree.is_within("331", "300"));
        assert!(tree.is_within("331", "331"));
        assert!(!tree.is_within("340", "330"));
        assert!(!tree.is_within("330", "331"));
    }

    #[test]
    fn a_flat_classification_is_one_level_of_its_row_classes() {
        let tree = ClassTree::of(&definition(Vec::new(), &["office", "lab"])).unwrap();
        assert!(!tree.is_hierarchical());
        assert_eq!(tree.depth(), 1);
        assert!(tree.contains("lab"));
        assert!(tree.is_within("lab", "lab"));
    }

    #[test]
    fn malformed_trees_are_refused() {
        let refused = |classes: Vec<ClassDefinition>, rows: &[&str]| {
            ClassTree::of(&definition(classes, rows)).unwrap_err()
        };
        let mut twice = cost_groups();
        twice.push(class("331", None, None));
        assert!(refused(twice, &["300"]).contains("declared twice"));
        let mut code = cost_groups();
        code.push(class("333", Some("331"), Some("330")));
        assert!(refused(code, &["300"]).contains("code `331`"));
        let orphan = vec![class("331", None, Some("330"))];
        assert!(refused(orphan, &["331"]).contains("undeclared parent"));
        let cycle = vec![class("a", None, Some("b")), class("b", None, Some("a"))];
        assert!(refused(cycle, &["a"]).contains("cycle"));
        let own = vec![class("a", None, Some("a"))];
        assert!(refused(own, &["a"]).contains("cycle"));
        assert!(refused(cost_groups(), &["999"]).contains("undeclared class `999`"));
        assert!(refused(vec![class(" ", None, None)], &[" "]).contains("blank id"));
        assert!(refused(vec![class("a", Some(""), None)], &["a"]).contains("blank code"));
    }

    #[test]
    fn a_property_name_reads_the_assigned_class_or_a_level() {
        let plain = ClassificationProperty::parse("cost-group").unwrap();
        assert_eq!(plain.level, None);
        let level = ClassificationProperty::parse("cost-group;level=2").unwrap();
        assert_eq!((level.classification, level.level), ("cost-group", Some(2)));
        assert_eq!(level.to_string(), "cost-group;level=2");
        for bad in [
            "x;level=0",
            "x;level=02",
            "x;level=",
            "x;depth=1",
            "x;level=1;y",
        ] {
            assert!(ClassificationProperty::parse(bad).is_err(), "{bad}");
        }
    }
}
