// Copyright 2017 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

// FIXME: remove following later
#![allow(dead_code)]

/// Bucket is an element of histogram.
///
/// A bucket count is the number of items stored in all previous buckets and the current bucket.
/// bucket numbers are always in increasing order.
///
/// A bucket value is the greatest item value stored in the bucket.
///
/// Repeat is the number of repeats of the bucket value, it can be used to find popular values.
///
struct Bucket {
    count: i64,
    upper_bound: Vec<u8>,
    lower_bound: Vec<u8>,
    repeats: i64,
}

impl Bucket {
    fn new(count: i64, upper_bound: Vec<u8>, lower_bound: Vec<u8>, repeats: i64) -> Bucket {
        Bucket {
            count: count,
            upper_bound: upper_bound,
            lower_bound: lower_bound,
            repeats: repeats,
        }
    }

    fn insert_repeated_item(&mut self) {
        self.count += 1;
        self.repeats += 1;
    }

    fn insert_item(&mut self, data: Vec<u8>) {
        self.upper_bound = data;
        self.count += 1;
        self.repeats = 1;
    }
}

/// Histogram represents statistics for a column or index.
#[derive(Default)]
pub struct Histogram {
    // column's id
    id: i64,
    // number of distinct values
    ndv: i64,
    buckets: Vec<Bucket>,
    // max number of values in per bucket. Notice: when a bucket's count is equal to
    // per_bucket_limit, only value equal with last value can been inserted into it.
    per_bucket_limit: i64,
    // max number of buckets
    buckets_num: usize,
}

impl Histogram {
    pub fn new(id: i64, buckets_num: usize) -> Histogram {
        let mut h = Histogram::default();
        h.per_bucket_limit = 1;
        h.buckets_num = buckets_num;
        h.id = id;
        h
    }

    fn iterate(&mut self, data: Vec<u8>) {
        if let Some(bucket) = self.buckets.last_mut() {
            // The new item has the same value as last bucket value, to ensure that
            // a same value only stored in a single bucket, we do not increase bucket
            // even if it exceeds per_bucket_limit.
            if bucket.upper_bound == data {
                bucket.insert_repeated_item();
                return;
            }
        }
        self.ndv += 1;
        if self.buckets.len() >= self.buckets_num && self.is_last_bucket_full() {
            self.merge_buckets();
        }

        if !self.is_last_bucket_full() {
            self.buckets.last_mut().unwrap().insert_item(data);
            return;
        }

        // create a new bucket and insert data
        let mut count = 1;
        if let Some(bucket) = self.buckets.last() {
            count += bucket.count
        }
        self.buckets.push(Bucket::new(count, data.clone(), data, 1));
    }

    // check whether the last bucket is full.
    fn is_last_bucket_full(&self) -> bool {
        if self.buckets.is_empty() {
            return true;
        }
        let count = if self.buckets.len() == 1 {
            self.buckets[0].count
        } else {
            let len = self.buckets.len();
            self.buckets[len - 1].count - self.buckets[len - 2].count
        };
        count >= self.per_bucket_limit
    }

    // It merges every two neighbor buckets.
    fn merge_buckets(&mut self) {
        self.buckets = {
            let mut buckets = Vec::with_capacity(self.buckets.len() / 2 + (self.buckets.len() & 1));
            let mut iter = self.buckets.drain(..);
            while let Some(first) = iter.next() {
                let bucket = match iter.next() {
                    Some(second) => Bucket::new(
                        second.count,
                        second.upper_bound,
                        first.lower_bound,
                        second.repeats,
                    ),
                    None => first,
                };
                buckets.push(bucket);
            }
            buckets
        };
        self.per_bucket_limit *= 2;
    }
}


#[cfg(test)]
mod test {
    use std::iter::repeat;
    use coprocessor::codec::datum;
    use coprocessor::codec::datum::Datum;
    use super::*;

    // This test was ported from tidb.
    #[test]
    fn test_histogram() {
        let buckets_num = 3;
        let mut hist = Histogram::new(1, buckets_num);
        assert_eq!(hist.buckets.len(), 0);

        for item in (0..3).map(Datum::I64) {
            let bytes = datum::encode_value(&[item]).unwrap();
            hist.iterate(bytes);
        }
        // b0: [0]
        // b1: [1]
        // b2: [2]
        assert_eq!(hist.buckets.len(), buckets_num);
        assert_eq!(hist.per_bucket_limit, 1);
        assert_eq!(hist.ndv, 3);

        // bucket is full now, need to merge
        let bytes = datum::encode_value(&[Datum::I64(3)]).unwrap();
        hist.iterate(bytes);
        // b0: [0, 1]
        // b1: [2, 3]
        assert_eq!(hist.per_bucket_limit, 2);
        assert_eq!(hist.buckets.len(), 2);
        assert_eq!(hist.ndv, 4);

        // push repeated item
        for item in repeat(3).take(3).map(Datum::I64) {
            let bytes = datum::encode_value(&[item]).unwrap();
            hist.iterate(bytes);
        }

        // b1: [0, 1]
        // b2: [2, 3, 3, 3, 3]
        assert_eq!(hist.per_bucket_limit, 2);
        assert_eq!(hist.buckets.len(), 2);
        assert_eq!(hist.ndv, 4);

        for item in repeat(4).take(4).map(Datum::I64) {
            let bytes = datum::encode_value(&[item]).unwrap();
            hist.iterate(bytes);
        }
        // b0: [0, 1]
        // b1: [2, 3, 3, 3, 3]
        // b2: [4, 4, 4, 4, 4]
        assert_eq!(hist.per_bucket_limit, 2);
        assert_eq!(hist.buckets.len(), 3);
        assert_eq!(hist.ndv, 5);

        // bucket is full now, need to merge
        let bytes = datum::encode_value(&[Datum::I64(5)]).unwrap();
        hist.iterate(bytes);
        // b0: [0, 1, 2, 3, 3, 3, 3]
        // b1: [4, 4, 4, 4, 4]
        // b2: [5]
        assert_eq!(hist.per_bucket_limit, 4);
        assert_eq!(hist.buckets.len(), 3);
        assert_eq!(hist.ndv, 6);
    }
}
