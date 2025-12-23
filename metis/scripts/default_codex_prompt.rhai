fn run() {
    let prompt = params[0];
    codex(prompt, |output| output)
}

run();
