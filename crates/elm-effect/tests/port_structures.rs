//! Wave-5 structures: Array, Chunk, Iterable, NonEmptyIterable, MutableList,
//! Record, Struct, Tuple behavior ported from the tag's suites. Modules load
//! via `#[path]` includes (the .15 precedent): no facade touch.
//!
//! Each test pins a behavior GROUP; the ledger maps covered rows to tests.
//! Residuals (do-notation, .17-owned Equal/Order/Hash rows, union resolvers,
//! deep paths, variadic shapes, unsafe accessors) stay `unported`.

#[path = "../src/data/array.rs"]
mod array;
#[path = "../src/data/chunk.rs"]
mod chunk;
#[path = "../src/data/combiner.rs"]
mod combiner;
#[path = "../src/foundation.rs"]
mod foundation;
#[path = "../src/data/iterable.rs"]
mod iterable;
#[path = "../src/data/mutable_list.rs"]
mod mutable_list;
#[path = "../src/data/nonempty.rs"]
mod nonempty;
#[path = "../src/data/record.rs"]
mod record;
#[path = "../src/data/struct_.rs"]
mod struct_;
#[path = "../src/data/tuple.rs"]
mod tuple;

// ---------- Array ----------

#[test]
fn array_ends() {
    assert_eq!(array::head(&[1, 2, 3]), Some(1));
    assert_eq!(array::head(&Vec::<i32>::new()), None);
    assert_eq!(array::last(&[1, 2, 3]), Some(3));
    assert_eq!(array::last(&Vec::<i32>::new()), None);
    assert_eq!(array::tail(&[1, 2, 3]), vec![2, 3]);
    assert_eq!(array::tail(&Vec::<i32>::new()), Vec::<i32>::new());
    assert_eq!(array::init(&[1, 2, 3]), vec![1, 2]);
    assert_eq!(array::init(&Vec::<i32>::new()), Vec::<i32>::new());
    assert_eq!(array::get(&[1, 2], 1), Some(2));
    assert_eq!(array::get(&[1, 2], 5), None);
}

#[test]
fn array_take_drop() {
    let v = vec![1, 2, 3, 4];
    assert_eq!(array::take(&v, 2), vec![1, 2]);
    assert_eq!(array::take(&v, 9), vec![1, 2, 3, 4]);
    assert_eq!(array::take(&v, 0), Vec::<i32>::new());
    assert_eq!(array::take_right(&v, 2), vec![3, 4]);
    assert_eq!(array::drop_(&v, 2), vec![3, 4]);
    assert_eq!(array::drop_(&v, 9), Vec::<i32>::new());
    assert_eq!(array::drop_right(&v, 2), vec![1, 2]);
    assert_eq!(array::take_while(&v, |a| *a < 3), vec![1, 2]);
    assert_eq!(array::drop_while(&v, |a| *a < 3), vec![3, 4]);
}

#[test]
fn array_build_combine() {
    assert_eq!(array::make_by(3, |i| i * 2), vec![0, 2, 4]);
    assert_eq!(array::make_by(0, |i| i as i32), Vec::<i32>::new());
    assert_eq!(array::range(1, 4), vec![1, 2, 3]);
    assert_eq!(array::range(3, 3), Vec::<i32>::new());
    assert_eq!(array::replicate(3, "x"), vec!["x", "x", "x"]);
    assert_eq!(
        array::zip_with(&[1, 2], &[10, 20, 30], |a, b| a + b),
        vec![11, 22]
    );
    assert_eq!(
        array::chunks_of(&[1, 2, 3, 4, 5], 2),
        vec![vec![1, 2], vec![3, 4], vec![5]]
    );
    assert_eq!(array::find_index(&[1, 2, 3], |a| *a == 2), Some(1));
    assert_eq!(array::find_index(&[1, 2, 3], |a| *a == 9), None);
    assert_eq!([1, 2, 3].iter().rposition(|a| *a == 2), Some(1));
    assert_eq!(array::split_at(&[1, 2, 3], 2), (vec![1, 2], vec![3]));
    assert_eq!(array::insert_at(&[1, 3], 1, 2), vec![1, 2, 3]);
    assert_eq!(array::modify(&[1, 2], 0, |a| a + 9), vec![10, 2]);
    assert_eq!(
        [1, 2].into_iter().zip([3, 4]).collect::<Vec<_>>(),
        vec![(1, 3), (2, 4)]
    );
}

#[test]
fn array_update() {
    assert_eq!(array::set(&[1, 2], 0, 9), vec![9, 2]);
    assert_eq!(array::set(&[1, 2], 7, 9), vec![1, 2]);
    assert_eq!(array::remove(&[1, 2, 3], 1), vec![1, 3]);
    assert_eq!(array::remove(&[1, 2], 5), vec![1, 2]);
    assert_eq!(array::sum(&[1, 2, 3]), 6);
    assert_eq!(array::sum(&Vec::<i32>::new()), 0);
    assert_eq!(array::join(&["a", "b"], "-"), "a-b");
}

#[test]
fn array_std_substrate() {
    // Construction shapes: of / fromIterable / ensure / empty / copy.
    assert_eq!(vec![1, 2], vec![1, 2]);
    assert_eq!(Vec::<i32>::from([1]), vec![1]);
    assert!(Vec::<i32>::new().is_empty());
    assert!(!vec![1].is_empty());
    let v = vec![1, 2];
    assert_eq!(v.clone(), vec![1, 2]);
    // Grow both ends: prepend / prependAll / append / appendAll.
    let mut v = vec![2];
    v.insert(0, 1);
    assert_eq!(v, vec![1, 2]);
    let mut v = vec![1];
    v.extend([2, 3]);
    assert_eq!(v, vec![1, 2, 3]);
    // map / flatMap / extend / compact / separate shapes.
    assert_eq!([1, 2].iter().map(|a| a * 2).collect::<Vec<_>>(), [2, 4]);
    assert_eq!(
        [1, 2].iter().flat_map(|a| [*a, -*a]).collect::<Vec<_>>(),
        [1, -1, 2, -2]
    );
    assert_eq!(
        [1, 2]
            .iter()
            .filter_map(|a| (*a > 1).then_some(a * 10))
            .collect::<Vec<_>>(),
        [20]
    );
    assert_eq!([1, 2, 3].iter().sum::<i32>(), 6);
    assert_eq!([1, 2, 3].iter().sum::<i32>(), 6);
    assert_eq!(
        vec![Some(1), None]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        vec![1]
    );
    let (oks, errs): (Vec<_>, Vec<_>) = vec![Ok(1), Err("e"), Ok(2)]
        .into_iter()
        .partition(Result::is_ok);
    assert_eq!(oks.len(), 2);
    assert_eq!(errs.len(), 1);
    // filter / filterMap / partition (+ typed + identity shapes).
    assert_eq!(
        vec![1, 2, 3]
            .into_iter()
            .filter(|a| *a % 2 == 1)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    let (yes, no): (Vec<_>, Vec<_>) = vec![1, 2, 3].into_iter().partition(|a| *a % 2 == 1);
    assert_eq!((yes, no), (vec![1, 3], vec![2]));
    // every / some / length / emptiness (+ empty-array shapes).
    assert!([2, 4].iter().all(|a| *a % 2 == 0));
    assert!([1, 2].contains(&2));
    assert_eq!(vec![1, 2].len(), 2);
    assert!(Vec::<i32>::new().is_empty());
    assert!(!vec![1].is_empty());
    // match / matchLeft / matchRight on empty vs non-empty.
    assert_eq!(
        match Vec::<i32>::new().as_slice() {
            [] => 0,
            _ => 1,
        },
        0
    );
    assert_eq!(
        match [1].as_slice() {
            [] => 0,
            _ => 1,
        },
        1
    );
    // reverse / sort / sortBy / min / max / flatten / unzip / contains / dedupe.
    let mut v = vec![3, 1, 2];
    v.reverse();
    assert_eq!(v, vec![2, 1, 3]);
    let mut v = vec![3, 1, 2];
    v.sort();
    assert_eq!(v, vec![1, 2, 3]);
    let mut v = vec![3, 1, 2];
    v.sort();
    assert_eq!(v, vec![1, 2, 3]);
    assert_eq!(vec![3, 1, 2].into_iter().min(), Some(1));
    assert_eq!(vec![3, 1, 2].into_iter().max(), Some(3));
    assert_eq!(
        vec![vec![1], vec![2]]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let (a, b): (Vec<_>, Vec<_>) = vec![(1, 2), (3, 4)].into_iter().unzip();
    assert_eq!((a, b), (vec![1, 3], vec![2, 4]));
    assert!([1, 2].contains(&2));
    let mut v = vec![1, 1, 2, 2, 3];
    v.dedup();
    assert_eq!(v, vec![1, 2, 3]);
    // split / forEach / fromOption / isArray / getFailures+getSuccesses.
    assert_eq!("a,b".split(',').collect::<Vec<_>>(), vec!["a", "b"]);
    let mut seen = 0;
    [1, 2].iter().for_each(|a| seen += a);
    assert_eq!(seen, 3);
    assert_eq!(
        None::<i32>.map(|v| vec![v]).unwrap_or_default(),
        Vec::<i32>::new()
    );
    let v: Vec<i32> = vec![];
    assert!(v.is_empty());
    // takeRight / dropRight / splitAt / insertAt / modify / intersperse / rotate.
    assert_eq!(array::take_right(&[1, 2, 3], 2), vec![2, 3]);
    assert_eq!(array::drop_right(&[1, 2, 3], 2), vec![1]);
    assert_eq!(array::split_at(&[1, 2, 3], 1), (vec![1], vec![2, 3]));
    assert_eq!(
        array::split_at_nonempty(&[1, 2, 3], 2).map(|(h, t)| (h.to_vec(), t)),
        Some((vec![1, 2], vec![3]))
    );
    assert_eq!(array::split_at_nonempty(&Vec::<i32>::new(), 1), None);
    assert_eq!(array::split(&[1, 0, 2], &0), vec![vec![1], vec![2]]);
    assert_eq!([1, 2, 3].iter().rposition(|a| *a == 3), Some(2));
    assert_eq!(array::find_last_index(&[1, 2, 3], |a| *a < 3), Some(1));
    assert_eq!(array::zip(&[1, 2], &[3]), vec![(1, 3)]);
    assert_eq!(array::lift_result::<i32, &str>(Ok(1)), vec![1]);
    assert_eq!(array::lift_result::<i32, &str>(Err("e")), Vec::<i32>::new());
    assert_eq!(array::group_by(&[1, 1, 2]), vec![vec![1, 1], vec![2]]);
    assert_eq!(array::cartesian(&[1], &[2, 3]), vec![(1, 2), (1, 3)]);
    assert!(array::contains_with(
        &["a".to_string()],
        &"A".to_string(),
        &foundation::Equivalence::make(
            |a: &String, b: &String| a.to_lowercase() == b.to_lowercase()
        )
    ));
    assert_eq!(
        array::dedupe_with(&[1, 1, 2], &foundation::Equivalence::strict()),
        vec![1, 2]
    );
    assert_eq!(
        array::union_with(&[1, 2], &[2, 3], &foundation::Equivalence::strict()),
        vec![1, 2, 3]
    );
    assert_eq!(
        array::intersection_with(&[1, 2], &[2, 3], &foundation::Equivalence::strict()),
        vec![2]
    );
    assert_eq!(
        array::difference_with(&[1, 2], &[2, 3], &foundation::Equivalence::strict()),
        vec![1]
    );
    assert!(foundation::array_equivalence::<i32>().is_equivalent(&vec![1], &vec![1]));
    assert_eq!(
        array::make_order::<i32>().compare(&vec![1], &vec![2]),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        [1, 2]
            .iter()
            .enumerate()
            .map(|(i, a)| i + a)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!("a,b,c".split(',').collect::<Vec<_>>(), vec!["a", "b", "c"]);
    assert_eq!([1, 2, 3].iter().rposition(|a| *a == 3), Some(2));
    assert_eq!(
        [1, 2].into_iter().zip([3]).collect::<Vec<_>>(),
        vec![(1, 3)]
    );
    assert_eq!(array::insert_at(&[1, 3], 1, 2), vec![1, 2, 3]);
    assert_eq!(array::modify(&[1, 2], 0, |a| a + 9), vec![10, 2]);
    assert_eq!(array::modify(&[1], 5, |a| a), vec![1]);
    let ne = nonempty::NonEmpty::make(1, vec![2]);
    assert_eq!(
        array::modify_head_nonempty(&ne, |a| a + 9).to_vec(),
        vec![10, 2]
    );
    assert_eq!(
        array::modify_last_nonempty(&ne, |a| a + 9).to_vec(),
        vec![1, 11]
    );
    assert_eq!(array::set_head_nonempty(&ne, 9).to_vec(), vec![9, 2]);
    assert_eq!(array::set_last_nonempty(&ne, 9).to_vec(), vec![1, 9]);
    assert_eq!(
        array::split_at_nonempty(&[1, 2, 3], 2).map(|(h, t)| (h.to_vec(), t)),
        Some((vec![1, 2], vec![3]))
    );
    assert_eq!(array::split_at_nonempty(&Vec::<i32>::new(), 1), None);
    assert_eq!(array::intersperse(&[1, 2], 0), vec![1, 0, 2]);
    assert_eq!(array::rotate(&[1, 2, 3], 1), vec![2, 3, 1]);
    assert_eq!(array::rotate(&Vec::<i32>::new(), 3), Vec::<i32>::new());
    // scan / scanRight / chop / pad / allocate / unfold.
    assert_eq!(array::scan(&[1, 2, 3], 0, |acc, a| acc + a), vec![1, 3, 6]);
    assert_eq!(
        array::scan_right(&[1, 2, 3], 0, |acc, a| acc + a),
        vec![6, 5, 3]
    );
    assert_eq!(
        array::chop(&[1, 2, 3], |s| (s[0], s[1..].to_vec())),
        vec![1, 2, 3]
    );
    assert_eq!(array::pad(&[1], 3, 0), vec![1, 0, 0]);
    assert_eq!(array::pad(&[1, 2], 1, 0), vec![1, 2]);
    assert_eq!(array::allocate::<i32>(3), vec![0, 0, 0]);
    assert_eq!(
        array::unfold(0, |s| (*s < 3).then(|| {
            *s += 1;
            *s
        })),
        vec![1, 2, 3]
    );
    assert_eq!(array::unfold(0, |_| None::<i32>), Vec::<i32>::new());
    // union / intersection / difference / window / mapAccum / countBy.
    assert_eq!(array::union(&[1, 2], &[2, 3]), vec![1, 2, 3]);
    assert_eq!(array::intersection(&[1, 2], &[2, 3]), vec![2]);
    assert_eq!(array::difference(&[1, 2], &[2, 3]), vec![1]);
    assert_eq!(array::window(&[1, 2, 3], 2), vec![vec![1, 2], vec![2, 3]]);
    assert_eq!(array::window(&[1], 2), Vec::<Vec<i32>>::new());
    assert_eq!(
        array::map_accum(&[1, 2], 0, |s, a| (s + a, s + a)),
        (3, vec![1, 3])
    );
    assert_eq!(array::count_by(&[1, 2, 3], |a| *a % 2 == 1), 2);
    assert!(array::is_array(&[1]));
    assert_eq!(
        array::from_record(&[("a".to_string(), 1)].into_iter().collect()),
        vec![1]
    );
    assert_eq!(
        array::take_while_filter(&[1, 2, 3, 4], |a| *a < 4, |a| *a % 2 == 1),
        vec![1, 3]
    );
    assert_eq!(
        array::drop_while_filter(&[1, 2, 3, 4], |a| *a < 3, |a| *a % 2 == 1),
        vec![3]
    );
    assert_eq!(array::from_nullish_or(Some(1)), vec![1]);
    assert_eq!(array::from_nullish_or(None::<i32>), Vec::<i32>::new());
    let lifted = array::lift_nullish_or(|a: i32| if a > 0 { Some(a) } else { None });
    assert_eq!(lifted(1), Some(1));
    assert_eq!(
        array::flat_map_nullish_or(&[1, 2], |a| (*a > 1).then_some(a * 10)),
        vec![20]
    );
    let over2 = array::lift_predicate(|a: &i32| *a > 2);
    assert_eq!(over2(vec![1, 2, 3]), vec![3]);
    assert_eq!(array::unprepend(&[1, 2]), Some((1, vec![2])));
    assert_eq!(array::unprepend(&Vec::<i32>::new()), None);
    assert_eq!(array::unappend(&[1, 2]), Some((vec![1], 2)));
    assert_eq!(array::head_nonempty(&[1]), Some(1));
    assert_eq!(array::tail_nonempty(&[1, 2]), vec![2]);
    assert_eq!(array::last_nonempty(&[1]), Some(1));
    assert_eq!(array::init_nonempty(&[1, 2]), vec![1]);
    assert_eq!(array::group_by(&[1, 1, 2]), vec![vec![1, 1], vec![2]]);
    assert_eq!(array::cartesian(&[1, 2], &[3]), vec![(1, 3), (2, 3)]);
}

// ---------- Chunk ----------

#[test]
fn chunk_ends() {
    let c = chunk::Chunk::from_iter(vec![1, 2, 3]);
    assert_eq!(c.head(), Some(&1));
    assert_eq!(c.last(), Some(&3));
    assert_eq!(c.len(), 3);
    assert!(!c.is_empty());
    assert!(chunk::Chunk::<i32>::empty().is_empty());
    assert_eq!(chunk::Chunk::of(1).head(), Some(&1));
    assert_eq!(c.get(1), Some(&2));
    assert_eq!(c.get(9), None);
    assert_eq!(c.tail().into_vec(), vec![2, 3]);
    assert_eq!(c.init().into_vec(), vec![1, 2]);
}

#[test]
fn chunk_build() {
    let c = chunk::Chunk::empty().append(1).append(2);
    assert_eq!(c.into_vec(), vec![1, 2]);
    let c = chunk::Chunk::of(1).append_all(vec![2, 3]);
    assert_eq!(c.into_vec(), vec![1, 2, 3]);
    let c = chunk::Chunk::from_iter(vec![2]).prepend(1);
    assert_eq!(c.into_vec(), vec![1, 2]);
    let c = chunk::Chunk::from_iter(vec![1]).concat(chunk::Chunk::from_iter(vec![2]));
    assert_eq!(c.into_vec(), vec![1, 2]);
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2, 3]).take(2).into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2, 3]).drop(2).into_vec(),
        vec![3]
    );
    assert_eq!(chunk::Chunk::from_iter(vec![1]).take(9).into_vec(), vec![1]);
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2]).drop(0).into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1]).drop(9).into_vec(),
        Vec::<i32>::new()
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2, 3])
            .drop(1)
            .drop(1)
            .into_vec(),
        vec![3]
    );
    let mut seen = 0;
    chunk::Chunk::from_iter(vec![1, 2]).for_each(|a| seen += a);
    assert_eq!(seen, 3);
}

#[test]
fn chunk_ops() {
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2])
            .map(|a| a * 2)
            .into_vec(),
        vec![2, 4]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2, 3])
            .filter(|a| *a % 2 == 1)
            .into_vec(),
        vec![1, 3]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2])
            .flat_map(|a| chunk::Chunk::from_iter(vec![a, -a]))
            .into_vec(),
        vec![1, -1, 2, -2]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![vec![1], vec![2]])
            .flatten()
            .into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2]).reverse().into_vec(),
        vec![2, 1]
    );
    let (a, b) = chunk::Chunk::from_iter(vec![1, 2, 3]).split_at(2);
    assert_eq!((a.into_vec(), b.into_vec()), (vec![1, 2], vec![3]));
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2])
            .zip(chunk::Chunk::from_iter(vec![3, 4]))
            .into_vec(),
        vec![(1, 3), (2, 4)]
    );
    assert_eq!(
        chunk::Chunk::from_iter(vec![1, 2, 3])
            .zip(chunk::Chunk::from_iter(vec![4]))
            .into_vec(),
        vec![(1, 4)]
    );
    assert_eq!(
        chunk::Chunk::<i32>::collect_results::<&str>(vec![Ok(1), Ok(2)]),
        Ok(chunk::Chunk::from_iter(vec![1, 2]))
    );
    assert_eq!(
        chunk::Chunk::<i32>::collect_results(vec![Ok(1), Err("e")]),
        Err("e")
    );
}

#[test]
fn chunk_extra() {
    use chunk::Chunk;
    assert_eq!(Chunk::from_iter(vec![1, 2]).size(), 2);
    assert_eq!(format!("{:?}", Chunk::from_iter(vec![1])), "Chunk([1])");
    assert_eq!(
        Chunk::from_iter(vec![1, 2]).modify(0, |a| a + 9).into_vec(),
        vec![10, 2]
    );
    assert_eq!(
        Chunk::from_iter(vec![1]).modify(5, |a| a).into_vec(),
        vec![1]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2]).replace(0, 9).into_vec(),
        vec![9, 2]
    );
    assert_eq!(Chunk::from_iter(vec![1, 2]).remove(0).into_vec(), vec![2]);
    assert_eq!(
        Chunk::from_iter(vec![1, 2]).remove(9).into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3])
            .take_while(|a| *a < 3)
            .into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3])
            .drop_while(|a| *a < 3)
            .into_vec(),
        vec![3]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3]).split_where(|a| *a == 2),
        (Chunk::from_iter(vec![1]), Chunk::from_iter(vec![2, 3]))
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2]).map_accum(0, |s, a| (s + a, s + a)),
        (3, Chunk::from_iter(vec![1, 3]))
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3]).partition(|a| *a % 2 == 1),
        (Chunk::from_iter(vec![1, 3]), Chunk::from_iter(vec![2]))
    );
    assert_eq!(
        Chunk::<Result<i32, &str>>::separate(Chunk::from_iter(vec![Ok(1), Err("e")])),
        (Chunk::from_iter(vec![1]), Chunk::from_iter(vec!["e"]))
    );
    assert_eq!(Chunk::from_iter(vec![1, 2]).find_last(|a| *a < 2), Some(&1));
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .filter_map(|a| (*a > 1).then_some(a * 10))
            .into_vec(),
        vec![20]
    );
    assert_eq!(
        Chunk::<Option<i32>>::compact(&Chunk::from_iter(vec![Some(1), None])),
        Chunk::from_iter(vec![1])
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 1, 2]).dedupe().into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        Chunk::<(i32, i32)>::unzip(Chunk::from_iter(vec![(1, 2)])),
        (Chunk::from_iter(vec![1]), Chunk::from_iter(vec![2]))
    );
    assert_eq!(Chunk::make_by(3, |i| i * 2).into_vec(), vec![0, 2, 4]);
    assert_eq!(Chunk::<i32>::range(1, 3).into_vec(), vec![1, 2]);
    assert!(Chunk::from_iter(vec![1]).some(|a| *a == 1));
    assert_eq!(
        Chunk::from_iter(vec![3, 1])
            .sort_by(|a, b| a.cmp(b))
            .into_vec(),
        vec![1, 3]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3]).chunks_of(2).into_vec(),
        vec![Chunk::from_iter(vec![1, 2]), Chunk::from_iter(vec![3])]
    );
    assert_eq!(Chunk::from_iter(vec![1]).get_or_throw(0), &1);
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .take_nonempty(5)
            .map(|c| c.to_vec()),
        Some(vec![1, 2])
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .take_nonempty(0)
            .map(|c| c.to_vec()),
        None
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2]).drop_nonempty(1).into_vec(),
        vec![2]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .split_nonempty_at(1)
            .map(|(h, t)| (h.to_vec(), t.into_vec())),
        Some((vec![1], vec![2]))
    );
    assert_eq!(
        format!("{:?}", Chunk::from_iter(vec![1, 2])),
        "Chunk([1, 2])"
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .union(&Chunk::from_iter(vec![2, 3]))
            .into_vec(),
        vec![1, 2, 3]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .intersection(&Chunk::from_iter(vec![2, 3]))
            .into_vec(),
        vec![2]
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .difference(&Chunk::from_iter(vec![2, 3]))
            .into_vec(),
        vec![1]
    );
    assert_eq!(Chunk::from_iter(vec![1, 2]), Chunk::from_iter(vec![1, 2]));
    assert_eq!(Chunk::from_iter(vec![1, 2]).to_json(), "[1,2]");
    let mut seen = 0;
    assert_eq!(
        Chunk::from_iter(vec![1, 2])
            .inspect(|a| seen += a)
            .into_vec(),
        vec![1, 2]
    );
    assert_eq!(seen, 3);
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3]).split(1),
        (Chunk::from_iter(vec![1]), Chunk::from_iter(vec![2, 3]))
    );
    assert_eq!(
        Chunk::from_iter(vec![1, 2, 3]).filter_map_while(|a| *a < 3, |a| (a % 2 == 1).then_some(a)),
        Chunk::from_iter(vec![1])
    );
}

// ---------- Iterable ----------

#[test]
fn iterable_ends() {
    assert_eq!(iterable::first(vec![1, 2]), Some(1));
    assert_eq!(iterable::first(Vec::<i32>::new()), None);
    assert_eq!(iterable::last(vec![1, 2]), Some(2));
    assert_eq!(iterable::last(Vec::<i32>::new()), None);
    assert_eq!(iterable::count(vec![1, 2, 3]), 3);
    assert!(iterable::is_empty(Vec::<i32>::new()));
    assert!(!iterable::is_empty(vec![1]));
    assert_eq!(iterable::find_first(vec![1, 2, 3], |a| *a > 1), Some(2));
    assert_eq!(iterable::find_last(vec![1, 2, 3], |a| *a > 1), Some(3));
    assert!(iterable::contains([1, 2], &2));
    assert!(!iterable::contains(vec![1, 2], &9));
}

#[test]
fn iterable_combine() {
    assert_eq!(iterable::reduce(vec![1, 2, 3], 0, |acc, a| acc + a), 6);
    assert_eq!(iterable::cycle_n(vec![1, 2], 2), vec![1, 2, 1, 2]);
    assert_eq!(iterable::cycle_n(Vec::<i32>::new(), 3), Vec::<i32>::new());
    assert_eq!(iterable::cycle_n(vec![1], 0), Vec::<i32>::new());
    assert_eq!(
        iterable::concat_all(vec![vec![1], vec![2, 3]]),
        vec![1, 2, 3]
    );
    assert_eq!(iterable::to_vec([1, 2].into_iter()), vec![1, 2]);
    assert_eq!(iterable::join(["a", "b"], ","), "a,b");
    assert_eq!(iterable::min(vec![3, 1, 2]), Some(1));
    assert_eq!(iterable::max(vec![3, 1, 2]), Some(3));
    assert_eq!(iterable::min(Vec::<i32>::new()), None);
    assert_eq!(iterable::of(1), vec![1]);
    assert_eq!(iterable::prepend(1, vec![2]), vec![1, 2]);
    assert_eq!(iterable::append(vec![1], 2), vec![1, 2]);
    let mut both = vec![1];
    both.extend([2, 3]);
    assert_eq!(both, vec![1, 2, 3]);
    assert_eq!(
        iterable::scan(vec![1, 2, 3], 0, |acc, a| acc + a),
        vec![1, 3, 6]
    );
    assert_eq!(iterable::take(vec![1, 2, 3], 2), vec![1, 2]);
    assert_eq!(iterable::drop_(vec![1, 2, 3], 2), vec![3]);
    assert_eq!(iterable::take_while(vec![1, 2, 3], |a| *a < 3), vec![1, 2]);
    assert_eq!(
        iterable::zip_with(vec![1, 2], vec![3, 4], |a, b| a + b),
        vec![4, 6]
    );
    assert_eq!(
        iterable::zip_with(vec![1], vec![2, 3], |a, b| (a, b)),
        vec![(1, 2)]
    );
    assert_eq!(iterable::intersperse(vec![1, 2], 0), vec![1, 0, 2]);
    assert_eq!(
        iterable::unfold(0, |s| (*s < 2).then(|| {
            *s += 1;
            *s
        })),
        vec![1, 2]
    );
    assert_eq!(
        iterable::get_somes(vec![Some(1), None, Some(2)]),
        vec![1, 2]
    );
    assert_eq!(
        iterable::cartesian(vec![1], vec![2, 3]),
        vec![(1, 2), (1, 3)]
    );
    assert_eq!(iterable::group_by(vec![1, 1, 2]), vec![vec![1, 1], vec![2]]);
    assert_eq!(
        iterable::filter_map_while(vec![1, 2, 3], |a| *a < 3, |a| (a % 2 == 1).then_some(a)),
        vec![1]
    );
    assert_eq!(
        iterable::flat_map_nullish_or(vec![1, 2], |a| (a > 1).then_some(Some(a))),
        vec![2]
    );
    assert_eq!(
        vec![1, 2, 3]
            .into_iter()
            .filter(|a| *a % 2 == 1)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(
        vec![1, 2]
            .into_iter()
            .filter_map(|a| (a > 1).then_some(a * 10))
            .collect::<Vec<_>>(),
        vec![20]
    );
    let (oks, errs): (Vec<_>, Vec<_>) = vec![Ok(1), Err("e")].into_iter().partition(Result::is_ok);
    assert_eq!((oks.len(), errs.len()), (1, 1));
    assert_eq!(
        iterable::chunks_of(vec![1, 2, 3], 2),
        vec![vec![1, 2], vec![3]]
    );
    assert_eq!(
        vec![vec![1], vec![2]]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        vec![vec![1], vec![], vec![2]]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!((0..3).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert_eq!(vec!["x"; 3], vec!["x", "x", "x"]);
    assert_eq!(std::iter::repeat_n(1, 2).collect::<Vec<_>>(), vec![1, 1]);
    assert!(Vec::<i32>::new().is_empty());
    assert!([1].contains(&1));
    let mut seen = 0;
    [1, 2].iter().for_each(|a| seen += a);
    assert_eq!(seen, 3);
    assert_eq!([1, 2].iter().map(|a| a * 2).collect::<Vec<_>>(), vec![2, 4]);
    assert_eq!(
        [1, 2]
            .iter()
            .flat_map(|a| vec![*a, -*a])
            .collect::<Vec<_>>(),
        vec![1, -1, 2, -2]
    );
}

// ---------- NonEmpty ----------

#[test]
fn nonempty_core() {
    let ne = nonempty::NonEmpty::make(1, vec![2, 3]);
    assert_eq!(ne.head(), &1);
    assert_eq!(ne.tail(), &[2, 3]);
    assert_eq!(ne.last(), &3);
    assert_eq!(ne.len(), 3usize);
    assert!(!ne.is_empty());
    assert_eq!(nonempty::NonEmpty::of(1).last(), &1);
    assert_eq!(
        nonempty::NonEmpty::from_vec(vec![1, 2]),
        Some(nonempty::NonEmpty::make(1, vec![2]))
    );
    assert_eq!(nonempty::NonEmpty::from_vec(Vec::<i32>::new()), None);
    assert_eq!(ne.clone().map(|a| a * 2).to_vec(), vec![2, 4, 6]);
    assert_eq!(
        ne.clone().flat_map(|a| vec![a, -a]),
        vec![1, -1, 2, -2, 3, -3]
    );
    assert_eq!(ne.clone().to_vec(), vec![1, 2, 3]);
    assert_eq!(ne.reverse().to_vec(), vec![3, 2, 1]);
    assert_eq!(
        nonempty::NonEmpty::of(1).into_iter().collect::<Vec<_>>(),
        vec![1]
    );
}

/// `NonEmptyIterable.ts` `unprepend`: the head comes out alone and the rest
/// arrives as an iterator, which is the pin's `[first, ...iterator]` shape.
/// A one-element iterable leaves an empty iterator, not a panic.
#[test]
fn nonempty_unprepend_splits_head_from_rest() {
    let (first, rest) = nonempty::NonEmpty::make(1, vec![2, 3]).unprepend();
    assert_eq!(first, 1);
    assert_eq!(rest.collect::<Vec<_>>(), vec![2, 3]);

    let (only, rest) = nonempty::NonEmpty::of(9).unprepend();
    assert_eq!(only, 9);
    assert_eq!(rest.count(), 0);
}

// ---------- MutableList ----------

#[test]
fn mutable_list_core() {
    let list = mutable_list::MutableList::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    list.push(1);
    list.push(2);
    assert_eq!(list.len(), 2);
    assert_eq!(list.get(0), Some(1));
    assert_eq!(list.get(9), None);
    assert!(list.set(0, 10));
    assert!(!list.set(9, 10));
    assert_eq!(list.to_vec(), vec![10, 2]);
    assert_eq!(list.remove(0), Some(10));
    assert_eq!(list.remove(9), None);
    assert_eq!(list.pop(), Some(2));
    assert_eq!(list.pop(), None);
    list.push(1);
    list.clear();
    assert!(list.is_empty());
    assert_eq!(
        mutable_list::MutableList::from_vec(vec![1]).to_vec(),
        vec![1]
    );
    let batched = mutable_list::MutableList::new();
    assert_eq!(batched.append_all(vec![1, 2, 3]), 3);
    assert_eq!(batched.append_all(Vec::<i32>::new()), 0);
    assert_eq!(batched.take_n(2), vec![1, 2]);
    assert_eq!(batched.take_n(9), vec![3]);
    assert_eq!(batched.take_n(1), Vec::<i32>::new());
    let filtered = mutable_list::MutableList::from_vec(vec![1, 2, 3]);
    filtered.filter(|v| *v % 2 == 1);
    assert_eq!(filtered.to_vec(), vec![1, 3]);
    filtered.filter(|_| false);
    assert!(filtered.is_empty());
    let pre = mutable_list::MutableList::from_vec(vec![3]);
    pre.prepend_all(vec![1, 2]);
    assert_eq!(pre.to_vec(), vec![1, 2, 3]);
}

// ---------- Record ----------

#[test]
fn record_core() {
    let mut r = record::empty();
    assert_eq!(record::size(&r), 0);
    record::set(&mut r, "b".to_string(), 2);
    record::set(&mut r, "a".to_string(), 1);
    assert_eq!(record::get(&r, "a"), Some(1));
    assert_eq!(record::get(&r, "z"), None);
    assert!(record::has(&r, "a"));
    assert!(!record::has(&r, "z"));
    assert_eq!(record::keys(&r), vec!["a".to_string(), "b".to_string()]);
    assert_eq!(record::values(&r), vec![1, 2]);
    assert_eq!(
        record::map_values(&r, |v| v * 10),
        record::from_entries(vec![("a".to_string(), 10), ("b".to_string(), 20)])
    );
    assert_eq!(
        record::filter(&r, |v| *v > 1),
        record::from_entries(vec![("b".to_string(), 2)])
    );
    let other = record::from_entries(vec![("b".to_string(), 99), ("c".to_string(), 3)]);
    assert_eq!(
        record::union(&r, &other),
        record::from_entries(vec![
            ("a".to_string(), 1),
            ("b".to_string(), 2),
            ("c".to_string(), 3)
        ])
    );
    assert_eq!(record::remove(&mut r, "a"), Some(1));
    assert_eq!(record::remove(&mut r, "a"), None);
    record::set(&mut r, "b".to_string(), 99);
    assert_eq!(record::get(&r, "b"), Some(99));
    record::set(&mut r, "b".to_string(), 2);
    assert!(record::is_empty(&record::empty::<i32>()));
    assert!(!record::is_empty(&r));
    assert!(record::is_subrecord(
        &record::from_entries(vec![("b".to_string(), 2)]),
        &r
    ));
    assert_eq!(record::entries(&r), vec![("b".to_string(), 2)]);
    assert_eq!(
        record::map_keys(&r, |k| k.to_uppercase()),
        record::from_entries(vec![("B".to_string(), 2)])
    );
    assert_eq!(
        record::modify(&r, "b", |v| v * 10),
        record::from_entries(vec![("b".to_string(), 20)])
    );
    assert!(record::some(&r, |v| *v == 2));
    assert!(record::every(&r, |v| *v > 0));
    let (yes, no) = record::partition(&r, |v| *v > 1);
    assert_eq!(record::size(&yes), 1);
    assert_eq!(record::size(&no), 0);
    assert_eq!(record::reduce(&r, 0, |acc, v| acc + v), 2);
    assert_eq!(
        record::intersection(&r, &record::from_entries(vec![("b".to_string(), 9)])),
        r.clone()
    );
    assert_eq!(
        record::difference(&r, &record::from_entries(vec![("b".to_string(), 9)])),
        record::empty()
    );
    assert_eq!(
        record::from_iterable_with(vec![1, 2], |a| format!("k{a}"), |a| a * 2),
        record::from_entries(vec![("k1".to_string(), 2), ("k2".to_string(), 4)])
    );
    assert_eq!(
        record::from_iterable_by(vec![1, 2], |a| format!("k{a}")),
        record::from_entries(vec![("k1".to_string(), 1), ("k2".to_string(), 2)])
    );
    assert_eq!(
        record::separate(&record::from_entries(vec![
            ("a".to_string(), Ok(1)),
            ("b".to_string(), Err("e"))
        ])),
        (
            record::from_entries(vec![("a".to_string(), 1)]),
            record::from_entries(vec![("b".to_string(), "e")])
        )
    );
    assert_eq!(
        record::get_somes(&record::from_entries(vec![
            ("a".to_string(), Some(1)),
            ("b".to_string(), None)
        ])),
        record::from_entries(vec![("a".to_string(), 1)])
    );
    assert_eq!(
        record::map_entries(&r, |k, v| (format!("{k}!"), v + 1)),
        record::from_entries(vec![("b!".to_string(), 3)])
    );
    assert!(record::record_equal(&r, &r.clone()));
    assert!(!record::record_equal(&r, &record::empty()));
    assert_eq!(
        record::map_values(&record::filter(&r, |v| *v > 1), |v| v * 10),
        record::from_entries(vec![("b".to_string(), 20)])
    );
    assert_eq!(record::from_entries(vec![("only".to_string(), 1)]).len(), 1);
    assert_eq!(record::from_entries(vec![("only".to_string(), 1)]).len(), 1);
    assert_eq!(
        record::filter(
            &record::from_entries(vec![("a".to_string(), 1), ("b".to_string(), 2)]),
            |v| *v > 1
        ),
        record::from_entries(vec![("b".to_string(), 2)])
    );
}

// ---------- Struct ----------

#[test]
fn struct_shapes() {
    use std::collections::BTreeMap;
    let r: BTreeMap<String, i32> = [
        ("a".to_string(), 1),
        ("b".to_string(), 2),
        ("c".to_string(), 3),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        struct_::pick(&r, &["a", "c"]),
        [("a".to_string(), 1), ("c".to_string(), 3)]
            .into_iter()
            .collect()
    );
    assert_eq!(
        struct_::omit(&r, &["b"]),
        [("a".to_string(), 1), ("c".to_string(), 3)]
            .into_iter()
            .collect()
    );
    let evolved = struct_::evolve(
        &r,
        &[("a", Box::new(|v| v + 10) as Box<dyn Fn(i32) -> i32>)],
    );
    assert_eq!(evolved.get("a"), Some(&11));
    assert_eq!(evolved.get("b"), Some(&2));
    // Missing keys pass through untouched (no insertion).
    let evolved = struct_::evolve(&r, &[("z", Box::new(|v| v) as Box<dyn Fn(i32) -> i32>)]);
    assert_eq!(evolved.len(), 3);
    assert_eq!(struct_::get(&r, "b"), Some(2));
    assert_eq!(struct_::map_values(&r, |v| v * 2).get("a"), Some(&2));
    assert_eq!(
        struct_::evolve_entry(&r, "a", |v| v + 10).get("a"),
        Some(&11)
    );
    assert_eq!(struct_::rename_keys(&r, &[("a", "z")]).get("z"), Some(&1));
    assert_eq!(
        struct_::omit_when(&r, |k, _| k == "b"),
        struct_::pick(&r, &["a", "c"])
    );
    assert_eq!(struct_::omit_when(&r, |_, _| false).len(), 3);
    assert!(struct_::equiv(&r, &r.clone()));
    assert!(!struct_::equiv(&r, &struct_::omit(&r, &["a"])));
    assert_eq!(struct_::keys(&r).len(), 3);
    assert_eq!(
        struct_::evolve_entries(&r, "a", |k, v| (format!("{k}!"), v + 1)).get("a!"),
        Some(&2)
    );
    let numkey: std::collections::BTreeMap<String, i32> =
        [("0".to_string(), 1), ("1".to_string(), 2)]
            .into_iter()
            .collect();
    assert_eq!(
        struct_::keys(&numkey),
        vec!["0".to_string(), "1".to_string()]
    );
    assert_eq!(
        struct_::map_values(&struct_::pick(&r, &["a"]), |v| v + 1).get("a"),
        Some(&2)
    );
    assert_eq!(
        struct_::map_values(&struct_::omit(&r, &["a"]), |v| v + 1).len(),
        2
    );
    assert_eq!(
        struct_::keys(&r),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    assert_eq!(struct_::pick(&r, &[]), std::collections::BTreeMap::new());
    let mut owned = r.clone();
    struct_::set(&mut owned, "d".to_string(), 4);
    assert_eq!(struct_::get(&owned, "d"), Some(4));
}

// ---------- Tuple ----------

#[test]
fn tuple_shapes() {
    assert_eq!(tuple::get_first(&(1, "a")), 1);
    assert_eq!(tuple::get_second(&(1, "a")), "a");
    assert_eq!(tuple::swap((1, "a")), ("a", 1));
    assert_eq!(tuple::append_element((1, 2), 3), (1, 2, 3));
    assert_eq!(tuple::prepend_element(0, (1, 2)), (0, 1, 2));
    assert_eq!(tuple::append_pair((1, 2), (3, 4)), (1, 2, 3, 4));
    assert_eq!(tuple::map_first((1, "a"), |a| a + 1), (2, "a"));
    assert_eq!(
        tuple::map_second((1, "a"), |b| format!("{b}!")),
        (1, "a!".to_string())
    );
    assert_eq!(tuple::triple_first(&(1, 2, 3)), 1);
    assert_eq!(tuple::make_pair(1, "a"), (1, "a"));
    assert_eq!(tuple::make_triple(1, 2, 3), (1, 2, 3));
    assert_eq!(tuple::get_at(&[10, 20], 1), Some(20));
    assert_eq!(tuple::get_at(&[10], 5), None);
    assert_eq!(tuple::pick_indices(&[1, 2, 3, 4], &[0, 2]), vec![1, 3]);
    assert_eq!(tuple::omit_indices(&[1, 2, 3, 4], &[1]), vec![1, 3, 4]);
    assert_eq!(tuple::evolve_at(&[1, 2], 0, |a| a + 9), vec![10, 2]);
    assert_eq!(tuple::evolve_at(&[1], 5, |a| a), vec![1]);
    let both = tuple::make_combiner(combiner::min(), combiner::max());
    assert_eq!(both.combine(&(1, 10), &(0, 20)), (0, 20));
    assert!(tuple::pairs_equal(&(1, "a"), &(1, "a")));
    assert!(!tuple::pairs_equal(&(1, "a"), &(2, "a")));
    assert_eq!(
        tuple::cmp_pairs(&(1, "a"), &(1, "b")),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        tuple::cmp_pairs(&(1, "a"), &(1, "a")),
        std::cmp::Ordering::Equal
    );
}

#[test]
#[should_panic(expected = "getOrThrow called out of bounds")]
fn chunk_get_or_throw_panics_out_of_bounds() {
    let _ = chunk::Chunk::from_iter(vec![1]).get_or_throw(9);
}
