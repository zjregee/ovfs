import os

TEST_POINT = "/mnt"
TEST_PRESET_PATHS = [
    ("./behavior_test_judge.py", True),
    ("./build_and_run_ovfs.sh", True),
    ("./file_behavior_test.py", True),
    ("./image.img", True),
    ("./install_and_run_vm.sh", True),
    ("./meta-data", True),
    ("./path_behavior_test.py", True),
    ("./seed.iso", True),
    ("./ubuntu-20.04.6-live-server-amd64.iso", True),
    ("./user-data", True),
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

def test_path():
    paths = list_paths()
    paths.sort()
    TEST_PRESET_PATHS.sort()
    assert paths == TEST_PRESET_PATHS

if __name__ == "__main__":
    test_path()
