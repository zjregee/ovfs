import paramiko
import time

hostname = "localhost"
port = 2222
username = "ubuntu"
password = "ubuntu"

ssh = paramiko.SSHClient()
ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())

# Wait for the VM to boot.
time.sleep(5 * 60)

try:
    ssh.connect(hostname=hostname, port=port, username=username, password=password)
    sudo_command = "sudo -S mount -t virtiofs myfs /mnt"

    stdin, stdout, stderr = ssh.exec_command(sudo_command)

    stdin.write(password + '\n')
    stdin.flush()

    output = stdout.read().decode('utf-8')
    errors = stderr.read().decode('utf-8')

    print("mount done")

finally:
    ssh.close()
