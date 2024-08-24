import os
from pathlib import Path

TEST_POINT = "/mnt"
TEST_PRESET_PATHS = [
    ("./behavior_test_judge.py", True),
    ("./file_behavior_test.py", True),
    ("./image.img", True),
    ("./install_and_run_vm.sh", True),
    ("./meta-data", True),
    ("./path_behavior_test.py", True),
    ("./seed.iso", True),
    ("./ubuntu-20.04.6-live-server-amd64.iso", True),
    ("./user-data", True),
]
TEST_NESTED_PATHS = [
    ("./dir1", False),
    ("./dir2", False),
    ("./dir3", False),
    ("./dir3/dir4", False),
    ("./dir3/dir5", False),
    ("./dir3/file3", True),
    ("./dir3/file4", True),
    ("./file1", True),
    ("./file2", True),
]

def list_paths():
    walked_entries = []
    for dirpath, dirnames, filenames in os.walk(TEST_POINT):
        rel_dir = os.path.relpath(dirpath, TEST_POINT)
        for dirname in dirnames:
            walked_entries.append((os.path.join(rel_dir, dirname), False))
        for filename in filenames:
            walked_entries.append((os.path.join(rel_dir, filename), True))
    return walked_entries

def create_paths():
    for path, is_file in TEST_NESTED_PATHS:
        if is_file:
            with open(Path(TEST_POINT) / path, "w") as f:
                f.write("This is a file.")
        else:
            os.makedirs(Path(TEST_POINT) / path, exist_ok=False)

def test_path():
    paths = list_paths()
    assert paths.sort() == TEST_PRESET_PATHS.sort()

def test_nested_path():
    create_paths()
    paths = list_paths()
    assert paths.sort() == (TEST_NESTED_PATHS + TEST_PRESET_PATHS).sort()

if __name__ == "__main__":
    test_path()
    test_nested_path()
