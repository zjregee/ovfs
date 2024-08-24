import paramiko
import sys
import time

hostname = "localhost"
port = 2222
username = "ubuntu"
password = "ubuntu"

ssh = paramiko.SSHClient()
ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())

# Wait for the VM to boot.
time.sleep(5 * 60)

ssh.connect(hostname=hostname, port=port, username=username, password=password)

mount_command = "sudo -S mount -t virtiofs myfs /mnt"
stdin, stdout, stderr = ssh.exec_command(mount_command)
stdin.write(password + '\n')
stdin.flush()
output = stdout.read().decode('utf-8')
errors = stderr.read().decode('utf-8')
exit_status = stdout.channel.recv_exit_status()
if exit_status != 0:
    print(errors)
    sys.exit(1)

test_command = "python3 /mnt/file_behavior_test.py"
stdin, stdout, stderr = ssh.exec_command(test_command)
output = stdout.read().decode('utf-8')
errors = stderr.read().decode('utf-8')
exit_status = stdout.channel.recv_exit_status()
if exit_status != 0:
    print(errors)
    sys.exit(1)

test_command = "python3 /mnt/path_behavior_test.py"
stdin, stdout, stderr = ssh.exec_command(test_command)
output = stdout.read().decode('utf-8')
errors = stderr.read().decode('utf-8')
exit_status = stdout.channel.recv_exit_status()
if exit_status != 0:
    print(errors)
    sys.exit(1)

ssh.close()
