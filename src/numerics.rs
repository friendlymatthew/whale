macro_rules! binop {
    ($stack:expr, $variant:ident, |$b:ident, $a:ident| $expr:expr) => {{
        let [Entry::Value(Value::$variant($b)), Entry::Value(Value::$variant($a))] =
            $stack.pop_array()?
        else {
            bail!(concat!("expected ", stringify!($variant), "s"))
        };
        $stack.push($expr);
    }};
}

macro_rules! cmpop {
    ($stack:expr, $variant:ident, |$b:ident, $a:ident| $expr:expr) => {{
        let [Entry::Value(Value::$variant($b)), Entry::Value(Value::$variant($a))] =
            $stack.pop_array()?
        else {
            bail!(concat!("expected ", stringify!($variant), "s"))
        };
        $stack.push(($expr) as i32);
    }};
}
