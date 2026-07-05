use corework::workflow::core::DataValue;
use corework::workflow::nodes::data::{ArrayLengthNode, GetFirstNode, GetLastNode, MakeArrayNode};
use std::collections::HashMap;

#[test]
fn array_nodes_compose_make_first_last_and_length() -> corework::error::Result<()> {
    let mut make_inputs = HashMap::new();
    make_inputs.insert("Element0".to_string(), DataValue::from_string("first"));
    make_inputs.insert("Element1".to_string(), DataValue::from_i64(2));
    make_inputs.insert("Element4".to_string(), DataValue::from_bool(true));

    let array_outputs = MakeArrayNode::new().evaluate(make_inputs)?;
    let array = array_outputs.get("Array").unwrap().clone();

    assert_eq!(array.array_len(), Some(3));
    assert_eq!(array.get_array_element(0).unwrap().as_str(), Some("first"));
    assert_eq!(array.get_array_element(1).unwrap().as_i64(), Some(2));
    assert_eq!(array.get_array_element(2).unwrap().as_bool(), Some(true));

    let mut first_inputs = HashMap::new();
    first_inputs.insert("Array".to_string(), array.clone());
    let first_outputs = GetFirstNode::new().evaluate(first_inputs)?;
    assert_eq!(first_outputs.get("IsValid").unwrap().as_bool(), Some(true));
    assert_eq!(
        first_outputs.get("Element").unwrap().as_str(),
        Some("first")
    );

    let mut last_inputs = HashMap::new();
    last_inputs.insert("Array".to_string(), array.clone());
    let last_outputs = GetLastNode::new().evaluate(last_inputs)?;
    assert_eq!(last_outputs.get("IsValid").unwrap().as_bool(), Some(true));
    assert_eq!(last_outputs.get("Element").unwrap().as_bool(), Some(true));

    let mut length_inputs = HashMap::new();
    length_inputs.insert("Array".to_string(), array);
    let length_outputs = ArrayLengthNode::new().evaluate(length_inputs)?;
    assert_eq!(length_outputs.get("Length").unwrap().as_i64(), Some(3));

    Ok(())
}
