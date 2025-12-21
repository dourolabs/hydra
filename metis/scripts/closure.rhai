//! Demonstrates a simple closure that captures mutable state.

let make_counter = || {
    let count = 0;

    || {
        count += 1;
        count
    }
};

let counter = make_counter();

print(counter());
print(counter());
