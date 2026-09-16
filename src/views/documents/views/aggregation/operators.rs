pub struct OperatorGroup {
    pub label: &'static str,
    /// (operator, what it does)
    pub operators: &'static [(&'static str, &'static str)],
}

pub const OPERATOR_GROUPS: &[OperatorGroup] = &[
    OperatorGroup {
        label: "Filter",
        operators: &[("$match", "Keep documents that match a query")],
    },
    OperatorGroup {
        label: "Transform",
        operators: &[
            ("$project", "Include, exclude, or compute fields"),
            ("$addFields", "Add or overwrite fields"),
            ("$set", "Add or overwrite fields"),
            ("$unset", "Remove fields"),
            ("$replaceRoot", "Promote an embedded document to the top level"),
            ("$replaceWith", "Replace each document with an expression"),
        ],
    },
    OperatorGroup {
        label: "Group",
        operators: &[
            ("$group", "Group documents and compute totals"),
            ("$bucket", "Group into ranges you define"),
            ("$bucketAuto", "Group into evenly sized ranges"),
        ],
    },
    OperatorGroup {
        label: "Join",
        operators: &[
            ("$lookup", "Join documents from another collection"),
            ("$unwind", "Output one document per array element"),
        ],
    },
    OperatorGroup {
        label: "Sort & limit",
        operators: &[
            ("$sort", "Order documents"),
            ("$limit", "Keep the first N documents"),
            ("$skip", "Skip the first N documents"),
        ],
    },
    OperatorGroup {
        label: "Output",
        operators: &[
            ("$out", "Replace a collection with the results"),
            ("$merge", "Merge the results into a collection"),
        ],
    },
    OperatorGroup {
        label: "Other",
        operators: &[
            ("$count", "Count documents into one field"),
            ("$facet", "Run several pipelines on the same input"),
            ("$sample", "Pick random documents"),
            ("$unionWith", "Append documents from another collection"),
            ("$redact", "Restrict content by document fields"),
            ("$graphLookup", "Recursively join documents"),
        ],
    },
];

pub const QUICK_START_OPERATORS: &[&str] = &["$match", "$group", "$project"];
