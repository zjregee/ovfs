import os

TEST_POINT = "/mnt"
TEST_TEXT = "OpenDAL: access data freely."

def test_file():
    path = os.path.join(TEST_POINT, "test_file.txt")
    with open(path, "w") as f:
        f.write(TEST_TEXT)
    with open(path, "r") as f:
        content = f.read()
        assert content == TEST_TEXT
    os.remove(path)

def test_file_append():
    path = os.path.join(TEST_POINT, "test_file_append.txt")
    with open(path, "w") as f:
        f.write(TEST_TEXT)
    with open(path, "a") as f:
        f.write(TEST_TEXT)
    with open(path, "r") as f:
        content = f.read()
        assert content == TEST_TEXT * 2
    os.remove(path)

def test_file_seek():
    path = os.path.join(TEST_POINT, "test_file_seek.txt")
    with open(path, "w") as f:
        f.write(TEST_TEXT)
    with open(path, "r") as f:
        f.seek(len(TEST_TEXT) // 2)
        content = f.read()
        assert content == TEST_TEXT[len(TEST_TEXT) // 2:]
    os.remove(path)

def test_file_truncate():
    path = os.path.join(TEST_POINT, "test_file_truncate.txt")
    with open(path, "w") as f:
        f.write(TEST_TEXT)
    with open(path, "w") as f:
        f.write(TEST_TEXT[:len(TEST_TEXT) // 2])
    with open(path, "r") as f:
        content = f.read()
        assert content == TEST_TEXT[:len(TEST_TEXT) // 2]
    os.remove(path)

if __name__ == "__main__":
    test_file()
    test_file_append()
    test_file_seek()
    test_file_truncate()
