FROM archlinux:base

RUN pacman -Syu --noconfirm && pacman -S --noconfirm --needed \
    base-devel \
    git \
    rust \
    sudo \
    python \
    ca-certificates

# makepkg must not run as root
RUN useradd -m builder && echo "builder ALL=(ALL) NOPASSWD: ALL" >> /etc/sudoers

WORKDIR /work
