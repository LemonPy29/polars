use crate::chunked_array::builder::aligned_vec_to_primitive_array;
use crate::prelude::*;
use arrow::array::{
    Array, ArrayData, BooleanArray, PrimitiveArray, PrimitiveArrayOps, StringArray, UInt32Array,
};
use arrow::buffer::MutableBuffer;
use arrow::util::bit_util;
use std::io::Write;
use std::mem;
use std::sync::Arc;

/// Forked snippet from Arrow. Remove on Arrow 3.0 release
pub(crate) fn take_no_null_boolean(arr: &BooleanArray, indices: &UInt32Array) -> Arc<BooleanArray> {
    assert_eq!(arr.null_count(), 0);
    let data_len = indices.len();
    let num_bytes = bit_util::ceil(data_len, 8);

    // fill with false bits
    let mut val_buf = MutableBuffer::new(num_bytes).with_bitset(num_bytes, false);
    let val_slice = val_buf.data_mut();
    (0..data_len).for_each(|i| {
        let index = indices.value(i) as usize;
        if arr.value(index) {
            bit_util::set_bit(val_slice, i)
        }
    });
    let nulls = indices.data_ref().null_buffer().cloned();
    let data = ArrayData::new(
        ArrowDataType::Boolean,
        indices.len(),
        None,
        nulls,
        0,
        vec![val_buf.freeze()],
        vec![],
    );
    Arc::new(BooleanArray::from(Arc::new(data)))
}

/// Forked snippet from Arrow. Only does not write to a Vec, but directly to aligned memory.
pub(crate) fn take_no_null_primitive<T: PolarsNumericType>(
    arr: &PrimitiveArray<T>,
    indices: &UInt32Array,
) -> Arc<PrimitiveArray<T>> {
    assert_eq!(arr.null_count(), 0);

    let data_len = indices.len();
    let mut values = AlignedVec::<T::Native>::with_capacity_aligned(data_len);
    for i in 0..data_len {
        let index = indices.value(i) as usize;
        let v = arr.value(index);
        values.inner.push(v);
    }
    let nulls = indices.data_ref().null_buffer().cloned();

    let arr = aligned_vec_to_primitive_array(values, nulls, Some(indices.null_count()));
    Arc::new(arr)
}

pub(crate) fn take_no_null_utf8(arr: &StringArray, indices: &UInt32Array) -> Arc<StringArray> {
    assert_eq!(arr.null_count(), 0);
    let data_len = indices.len();

    let offset_len_in_bytes = (data_len + 1) * mem::size_of::<i32>();
    let mut offset_buf = MutableBuffer::new(offset_len_in_bytes);
    offset_buf
        .resize(offset_len_in_bytes)
        .expect("out of memory");
    let offset_typed = offset_buf.typed_data_mut();

    let mut length_so_far = 0;
    offset_typed[0] = length_so_far;
    let mut values_buf;

    let nulls;

    // the two loops are ~30% faster then a single loop in benchmarks.
    offset_typed
        .iter_mut()
        .skip(1)
        .enumerate()
        .for_each(|(idx, offset)| {
            let index = indices.value(idx) as usize;
            let s = arr.value(index);
            length_so_far += s.len() as i32;
            *offset = length_so_far;
        });
    values_buf = MutableBuffer::new(length_so_far as usize);

    // both 0 nulls
    if arr.null_count() == indices.null_count() {
        (0..data_len).for_each(|idx| {
            let index = indices.value(idx) as usize;
            let s = arr.value(index);
            values_buf.write_all(s.as_bytes()).expect("enough capacity");
        });
        nulls = None
    } else {
        (0..data_len).for_each(|idx| {
            if indices.is_valid(idx) {
                let index = indices.value(idx) as usize;
                let s = arr.value(index);
                values_buf.write_all(s.as_bytes()).expect("enough capacity");
            }
        });
        nulls = indices.data_ref().null_buffer().cloned();
    }

    let mut data = ArrayData::builder(ArrowDataType::Utf8)
        .len(data_len)
        .add_buffer(offset_buf.freeze())
        .add_buffer(values_buf.freeze());
    if let Some(null_buffer) = nulls {
        data = data.null_bit_buffer(null_buffer);
    }
    Arc::new(StringArray::from(data.build()))
}