FROM fedora:latest
RUN dnf -y update && dnf -y install openssh-server openssh-clients
RUN mkdir /var/run/sshd
RUN echo 'root:toor' | chpasswd
RUN sed -i 's/#PasswordAuthentication yes/PasswordAuthentication yes/g' /etc/ssh/sshd_config
RUN sed -i 's/#PermitRootLogin prohibit-password/PermitRootLogin yes/g' /etc/ssh/sshd_config
# Ensure ForceCommand is not set, which could result in scp not working
RUN sed -i 's/^ForceCommand/#ForceCommand/g' /etc/ssh/sshd_config
RUN ssh-keygen -A
COPY authorized_keys /root/.ssh/authorized_keys
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D"]
