/*
 * The in-process server transport (docs/design/wine-unix.md, section 7).
 *
 * The client side of what the 02xx patches change in server.c and in the
 * server: the start, the queue of fds the server sends the client, and the
 * wake queue of the threads that wait on their wakeup pipes.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#include "tempwind_private.h"

/* The server's (server/request.c). */
void inproc_server_init( int msg_fd );

/* Starts the server in the calling thread, in place of server_connect: what
 * the server's main would run, then the first process and thread. Returns
 * the client's fd_socket, the write end of the pipe that is the process's
 * msg_fd; nothing is written to it. */
int inproc_server_start( void )
{
    int msg_pipe[2];

    if (pipe( msg_pipe ) == -1)
    {
        perror( "tempwind: inproc: pipe" );
        abort();
    }
    inproc_server_init( msg_pipe[0] );
    return msg_pipe[1];
}

/* ---- fds from the server: send_client_fd queues, wine_server_receive_fd takes ---- */

#define FD_QUEUE_SIZE 16  /* a request sends at most a few */

static struct { int fd; unsigned int handle; } fd_queue[FD_QUEUE_SIZE];
static unsigned int fd_queue_first, fd_queue_count;

void inproc_client_fd_push( int fd, unsigned int handle )
{
    unsigned int i = (fd_queue_first + fd_queue_count) % FD_QUEUE_SIZE;

    if (fd_queue_count == FD_QUEUE_SIZE)
        tw_unsupported( "inproc: more fds from the server than the client has taken (queue full)" );
    fd_queue[i].fd = fd;
    fd_queue[i].handle = handle;
    fd_queue_count++;
}

/* The oldest queued fd and its handle, or -1 if the queue is empty. */
int inproc_client_fd_pop( unsigned int *handle )
{
    int fd;

    if (!fd_queue_count) return -1;
    fd = fd_queue[fd_queue_first].fd;
    *handle = fd_queue[fd_queue_first].handle;
    fd_queue_first = (fd_queue_first + 1) % FD_QUEUE_SIZE;
    fd_queue_count--;
    return fd;
}

/* ---- waits on the wakeup pipes ---- */

/* One queue for every thread's wakeup pipe: a woken thread reads its own
 * pipe and waits again if it is empty. */
static struct green_queue wakeup_queue = { .what = "its server wakeup pipe (a server wait)" };

/* A wait_fd read got EAGAIN: wait until send_thread_wakeup writes. */
void inproc_wait_wakeup( void )
{
    green_wait( &wakeup_queue, NULL, 0 );
}

/* send_thread_wakeup wrote to a wakeup pipe. */
void inproc_wake_waiters( void )
{
    green_wake_queue( &wakeup_queue, 1 );
}
