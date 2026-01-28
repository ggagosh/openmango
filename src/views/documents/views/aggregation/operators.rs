pub struct OperatorGroup {
    pub label: &'static str,
    pub operators: &'static [&'static str],
}

const FILTER_OPERATORS: &[&str] = &["$match"];
const TRANSFORM_OPERATORS: &[&str] =
    &["$project", "$addFields", "$set", "$unset", "$replaceRoot", "$replaceWith"];
const GROUP_OPERATORS: &[&str] = &["$group", "$bucket", "$bucketAuto"];
const JOIN_OPERATORS: &[&str] = &["$lookup", "$unwind"];
const SORT_LIMIT_OPERATORS: &[&str] = &["$sort", "$limit", "$skip"];
const OUTPUT_OPERATORS: &[&str] = &["$out", "$merge"];
const OTHER_OPERATORS: &[&str] =
    &["$count", "$facet", "$sample", "$unionWith", "$redact", "$graphLookup"];

pub const OPERATOR_GROUPS: &[OperatorGroup] = &[
    OperatorGroup { label: "Filter", operators: FILTER_OPERATORS },
    OperatorGroup { label: "Transform", operators: TRANSFORM_OPERATORS },
    OperatorGroup { label: "Group", operators: GROUP_OPERATORS },
    OperatorGroup { label: "Join", operators: JOIN_OPERATORS },
    OperatorGroup { label: "Sort & Limit", operators: SORT_LIMIT_OPERATORS },
    OperatorGroup { label: "Output", operators: OUTPUT_OPERATORS },
    OperatorGroup { label: "Other", operators: OTHER_OPERATORS },
];

pub const QUICK_START_OPERATORS: &[&str] = &["$match", "$group", "$project"];
