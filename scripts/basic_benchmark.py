import os
import time
import hashlib

def generate_random_data(size):
    return os.urandom(size)

def write_file(filename, data):
    with open(filename, 'wb') as f:
        f.write(data)

def read_file(filename):
    with open(filename, 'rb') as f:
        return f.read()

def compute_checksum(data):
    return hashlib.sha256(data).hexdigest()

def test_read_write(file_sizes, iterations):
    filename = 'test_file.bin'
    for file_size in file_sizes:
        data_to_write = generate_random_data(file_size)
        write_checksum = compute_checksum(data_to_write)
        start_time = time.time()
        for i in range(iterations):
            write_file(filename, data_to_write)
            read_data = read_file(filename)
            read_checksum = compute_checksum(read_data)
            if write_checksum != read_checksum:
                print(f"Data consistency check FAILED on iteration {i+1}!")
                break
        end_time = time.time()
        total_time = end_time - start_time
        print(f"Total time for {iterations} iterations: {total_time:.4f} seconds")
        os.remove(filename)

def test_create_delete(iterations):
    filename = 'test_file.bin'
    data_to_write = generate_random_data(1)
    start_time = time.time()
    for i in range(iterations):
        write_file(filename, data_to_write)
        os.remove(filename)
    end_time = time.time()
    total_time = end_time - start_time
    print(f"Total time for {iterations} iterations: {total_time:.4f} seconds")

if __name__ == "__main__":
    file_size = [4, 64, 1024, 4096]
    iterations = 1000
    test_read_write(file_size, iterations)
    test_create_delete(iterations)
