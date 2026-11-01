# ~/.bashrc
[[ $- != *i* ]] && return
[ -f /usr/share/bash-completion/bash_completion ] && . /usr/share/bash-completion/bash_completion
alias ls='ls --color=auto' ll='ls -lah --color=auto' grep='grep --color=auto'
PS1='\[\e[38;5;111m\]\u@\h\[\e[0m\] \[\e[38;5;245m\]\w\[\e[0m\] \$ '
