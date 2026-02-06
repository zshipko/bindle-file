import subprocess
import os
import hashlib

def get_hash(data):
    return hashlib.sha256(data).hexdigest()

def run_test():
    test_file = "compat_test.bndl"
    secret_content = b"Consistency is the playground of the gods."
    with open("input.txt", "wb") as f:
        f.write(secret_content)

    print("--- Phase 1: Rust Create -> C Read ---")
    # 1. Create with Rust
    subprocess.run(["cargo", "run", "--",  "add",test_file, "msg", "input.txt", "--compress"], check=True)
    
    # 2. Read with C (Assuming you compiled the C example to ./bindle_c)
    result_c = subprocess.run(["./bindle_c",  "cat", test_file, "msg"], capture_output=True)
    
    if get_hash(result_c.stdout) == get_hash(secret_content):
        print("✅ SUCCESS: C successfully read Rust-compressed data.")
    else:
        print("❌ FAIL: C output does not match original content.")

    print("\n--- Phase 2: C Create -> Rust Read ---")
    # 3. Use C to add a different file (Assuming your C binary has an 'add' command)
    subprocess.run(["./bindle_c", "add", test_file,  "c_msg", "input.txt", "1"], check=True) # 1 for compress
    
    # 4. Use Rust to list and verify
    result_rust = subprocess.run(["cargo", "run", "--", "cat", test_file,  "c_msg"], capture_output=True)

    if get_hash(result_rust.stdout) == get_hash(secret_content):
        print("✅ SUCCESS: Rust successfully read C-compressed data.")
    else:
        print("❌ FAIL: Rust output does not match.")

if __name__ == "__main__":
    run_test()
