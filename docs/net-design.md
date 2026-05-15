# The architecture of the network stack of Gosub

This document is kind of a blog post / brain dump of the current state of the network architecture that currently
is implemented in this respository. Even though fetching resources over the network is easy enough with crates likes 
`reqwest`, there is much more that needs to be taken care of when designing a network stack for browser.


## Starting with the beginning

Let assume a very naive network stack:

```rust

async fn fetch(url: &str) -> Result<String, Error> {
    let response = reqwest::get(url).await?;
    let body = response.text().await?;
    Ok(body)
}

```

This, in theory, should be enough to fetch a resource over the network. But there are a few problems with this approach:

1. **Blocking**: The above function is asynchronous, but it still blocks the current thread until the request is 
   complete. In a browser, we want to be able to fetch resources without blocking the main thread.
2. **Memory usage**: The above function reads the entire response body into memory before returning it. This can be a 
   problem for large resources, as it can lead to high memory usage.
3. **Coalsecing**: Suppose you have multiple tabs open in your browser, and each tab is trying to fetch resources from 
   the same domain. If each tab makes its own request, this can lead to a lot of redundant network traffic. Instead, we 
   want to be able to coalesce requests for the same resource, so that only one request is made and the response is 
   shared among all the tabs that need it.
4. **Cancellation** : In a browser, users can navigate away from a page or close a tab at any time. If a request is in 
   progress when this happens, we want to be able to cancel the request to avoid wasting network resources
5. **Priority**: Some resources are more important than others. For example, the HTML of a page is more important than 
   an image on the page. We want to be able to prioritize requests so that more important resources are fetched first.
6. **Error handling**: Network requests can fail for a variety of reasons (e.g., network errors, server errors, etc.). 
   We want to be able to handle these errors gracefully and retry requests when appropriate.

These, and other problems we will solve by using a more complex setup that involves multiple components working 
together.


## Reader
Every time we read data from the network, we take care of a few things:

1. We take into account the "cancellation" of the request. If the request is cancelled, we stop reading data from 
   the network immediately.
2. We keep in mind an idle timeout. If we don't receive any data from the network for a certain amount of time, we 
   consider the request to be stalled and we cancel it.
3. We keep in mind a total timeout. If the request takes too long to complete, we cancel it.
4. Sometimes, but not always, we want to limit the amount of data we read from the network. This is useful for 
   example when we are downloading a large file and we want to limit the amount of data we read into memory at once.

## I/O Thread

First, we move all the IO related task away from the main thread. For this we make a new thread that will be the 
`I/O Thread`. This thread will be responsible for all the network related tasks, and will communicate with the main
thread using channels.

The `io_runtime.rs` file contains the code for the I/O thread. It spawns a new thread and returns a handle to it. This 
handle can be used to send tasks to the I/O thread. It contains a channel (`io_handle.subscribe()`) that can be passed
around different components to send tasks to the I/O thread.

The I/O Thread works by using an `IoRouter` to route requests to per-zone `Fetcher` instances, spawning them on
first use. You submit a `FetchRequest` via `IoCommand::Fetch` to the I/O thread, which routes it to the appropriate
zone's fetcher. That's basically it.

The actual work will be done in the fetcher itself, leaving the I/O thread loop pretty simple.


## Fetcher
The `fetcher.run()` function is the main loop of the fetcher. It is responsible for processing requests.
It works by fetching a request from the priority queues, and process it. If none a present, it will sleep until a new
request has been submitted through `submit()`.

### Priority queues
Since some requests are more important than others, we need to be able to prioritize requests. For this we use multiple
priority queues. Each queue has a different priority level, and requests are processed in order of priority. The 
priority levels are:

- High
- Normal
- Low
- Idle

The scheduler will try and fetch requests from the highest priority queue first. If there are no requests in the 
highest priority queue, it will try and fetch from the next highest priority queue, and so on. Note that it will not 
starve lower priority queues. 
If a lower priority queue has been waiting for a while, it will be given a chance to be processed, even when higer 
priority queues are not empty.

Once the fetcher has selected a request to process, it will check if there is already an ongoing request for the same 
URL. If so, we can coalesce the requests, and just add the new request to the list of listeners for the ongoing request.
This way, when the ongoing request completes, all the listeners will be notified with the result. 

To find out if we can coalesce, we need to generate some kind of key for the request. This takes elements like the URL, 
method, headers, etc. Once we have the key, we can check if there is an ongoing request with the same key. If so, we 
can coalesce the requests. If not, we can start a new request.

There is some small thing we need to consider when coalescing requests. Some consumers can requests to be fetch as 
streaming, others as buffered. Streaming requests will return a stream of data, while buffered requests will return the 
entire response body as a single buffer.

Each unique non-coalseced request will be represented by a `FetchInflightEntry`. This struct will contain the request,
the list of listeners, and the state of the job. This system will also take care of streamable vs buffered requests,
and can serve both types of requests from the same inflight job.

If the request wasn't coalescable, we can fetch the actual request. This is done by spawning a new task `Fetcher`. The 
first thing it does is to check if we have too many requests to a single origin. There can be 100 independent requests 
running, but we only allow a number of requests running simultaneously to a single origin. This is to avoid overloading
the server, and to avoid running into issues with browsers that limit the number of connections to a single origin.

In total, the fetcher will limit the total amount of connections that can be open at the same time, as well as the 
amount of connections that can be open to a single origin.

At this point we can actualy fetch the request. If the request is a streaming request, we will use `perform_streaming`, 
otherwise we will use the `perform_buffered` function.

There is a bit of a subtlety here. If the request is a streaming request, it could be possible that we coalesce a 
buffered request with a streaming request. Same goes the other way: when we have a buffered request, we could coalesce 
a streaming request with it.

The last thing is not a problem: once we have the buffered response, we can just create a stream from it, and send it 
to the streaming listeners. The first case is a bit more tricky. If we have a streaming request, we can't just buffer 
the response and send it to the buffered listeners. This can ONLY be done if the stream hasn't started yet. Since we 
don't keep the entire response in memory, we can't just buffer it and send it to the buffered listeners. In that case, 
we can't coalesce the requests, and we need to start a new request for the buffered listener.

Both the `perform_buffered` and `perform_streaming` function will return a `FetchResult`. This result will be sent to 
all the listeners of the request, and the inflight job will be removed. That will finish up the `Fetcher` spawned task.

## Inflights and waiters

Before we go into more detail of the buffered and streaming fetchers, let's take a look at the `FetchInflightEntry`
struct. This struct is responsible for keeping track of the state of a request. It contains the request, the list of
listeners, and wether or not streaming is being used. The most important part is the set of listeners, also called
`Waiter`s. It allows you to register channels that will receive the `FetchResult` from the `perform_buffered` and
`perform_streaming` functions.

Most of the work is done in the `Waiter.finish` function. Here is where we try to duplicate the result to all the 
listeners. 

`FetchResult::Buffered` will be sent to all the listeners as-is.
`FetchResult::Stream` is a bit more tricky. We will create cloned stream results for all the listeners that 
requested a streaming response. Then, we will fetch the stream ourselves into a buffer, and send the buffer to all the 
buffered listeners. This way, we can serve both streaming and buffered requests from the same inflight job.


### fetch-response_complete
This function is responsible for fetching the complete response. It will first fetch the top of the request through 
`fetch_response_top`. Then it will stream the rest of the response into a buffer, and return the complete response.

### fetch_response_top
This function is responsible for fetching the top of the response. These are the headers of the response, plus the
initial 5KB of the body. With this information, the client (or engine) can decide how to treat the response. For 
instance, if the response is a HTML document, the engine could decide to send the stream to the HTML5 parser etc.

There are a small issues:
If you read the first 5KB of the body, you can't read the rest of the body from the stream, it's possible that you
have read more than 5KB. Since we only want to have 5KB, we need to "unread" the extra bytes. This is done by dumping
the extra bytes into a 'excess' buffer.

When we return the stream back to the caller, we will not return the existing stream (since that already read the 
excess bytes), but we recreate a new stream that first reads the excess bytes, and then reads the rest of the body 
from the original

```
    //
    //  |--- Peek buffer ---|---- Excess buffer ----| ---- body stream ----|
    //                                              ^ stream starts here
    //                      ^  new body stream "rereads" the excess buffer and starts here
```

This means that now the FetchResult::Stream contains the 'peek_buf' and the reader that reads DIRECTLY behind
the peek_buf. (resulting in a small bit over 'rereading' the excess buffer).

### perform_streaming
This function calls `fetch_response_top` to get the top of the response and turns the result into a
`FetchResult::Stream`.

### perform_buffered
This will simply call the `fetch_response_complete` function, and turns the result into a `FetchResult::Buffered`.



## Sending notification during reading of data
The network stack will send notifications on certain events during reading. For instance, when a request has been 
queued, when it has started, when it is in progress of reading, etc. The fetch functionality uses `NetEvent` enum for 
this. But ultimately, we want to send other type of events to the client. In order to facilitate this, we use an 
"observer" system. This system is passed around to the actual fetch functions, and can be used to send events to the 
client. They will convert any `NetEvent` into a more suitable event for the client, and send it through a channel.

When reading a streaming response, we will send a `NetEvent::Progress` event every time we read a chunk of data. This 
way, the client can be notified of the progress of the request. For this, we wrap the read stream into a 
`ProgressReader`, which does nothing more than read the stream, and send a `NetEvent::Progress` events. Besides that,
it also takes care of idle timeouts, and total timeouts, cancellation and max size limits.


## Sharedbody
Sometimes, we don't return directly a stream, but a `SharedBody`. This works the same way as a normal stream, but it 
can be read by multiple consumers. You can call `subscribe_stream()` (or `subscribe_with_cap()` for a bounded variant)
to get a new stream that reads the same data as the original stream. This is useful when you have multiple consumers
that want to read the same stream, but you don't want to read the stream multiple times from the network.