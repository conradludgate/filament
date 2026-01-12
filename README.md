# Federtaed sync engine based on MLS, Paxos, iroh, and CRDTs.

With a federated set of untrusted backup servers, synchronise a document between multiple devices
with full encryption and local-first support. Devices sync periodically with the federated backup servers
and the shared document is eventually consistent and rebuilt using extensible CRDTs.

## Design

![](readme/sync.png)

Devices send messages through a small set of federated servers. These servers are untrusted but expected
to maintain availability and to not withhold messages. With enough federated servers controlled by seperate owners,
availability and withholding of messages is not a pracitcal attack. These servers can be changes over time and at any time.

All device messages are signed and encrypted using MLS to ensure forward secrecy and post compromise secrecy.
This also prevents the federated servers from knowing the contents of the messages, or to forge messages. The
federated servers however do understand the group and who is in it. 

The design of MLS allows messages to be encrypted and decrypted asynchronously, 
which allows for a local-first architecture when combined with CRDTs.

Changes to the group, such as updating keys or updating membership must be done synchronously and must be strongly serialised.
The devices and the federated servers take part in Paxos to form consensus on the strict order of MLS commits.

Initial synchronisation is performed in a p2p fashion, using iroh. When a device is added to the group,
the group member encodes the current document state and sends it to the peer directly. This allows
new members to know the current state and to keep it synchronised without needing to know the entire document
history.

Read-only membership can be achieved by extensions to the MLS group. Messages they send will be ignored
unless a group admin allows that member to send messages.

### MLS ([Messaging Layer Security](https://en.wikipedia.org/wiki/Messaging_Layer_Security))

Since synchronising devices requires sharing messages between multiple devices, 
we use MLS to provide the efficient key exchange protocol among those devices.
It allows to efficiently and securely add/remove/update members in the sync group.

A group is created for each document, and MLS allows easily creating new groups from an existing group
for new documents.

We use MLS' extensibility to add a few extra features:
1. Add/Remove federated sync nodes from the group
2. Add/Remove admins from the group.

Only admins can change the membership properties of the group.

### Paxos

Most of messages sent through MLS can work asynchrously, commits to the group must all
be processed in the same order. To provide the serialisation of commits, and to ensure all
members can see every commit, we use the Paxos consensus protocol.

* Group members are all "Learners" of the protocol. They will receive updates about new commits.
* A group member trying to commit will be a "Proposer" of the protocol.
* The federated backup servers will be the "Acceptors" of the protocol.

When a new device is to be added to the sync group, an existing device will propose this and send the commit
to all the federated acceptors. If all the devices observe a quorom of acceptance messages, then they know that this
commit has been officiall accepted and add the new device to the group.

The federated acceptors are community run and know which devices are in the group, but they do not know
what messages are sent through the group. They might prevent messages from being sent, but users are recommended
to use multiple sync servers with multiple administrators to ensure availability.

### [iroh](https://www.iroh.computer/)

All device communication occurs with Iroh. While all MLS messages go over the federation servers, adding a new
device to a group requires an initial sync. This initial sync is done peer-to-peer using Iroh.

To simplify the setup, all federated sync servers also use iroh when communicating to devices, and these federated servers
can double up as [iroh relays](https://docs.iroh.computer/concepts/relays) to provide holepunching or resilient transport
between devices over multiple NATs.

### CRDTs ([Conflict-free Replicated Data Types](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type))

While group commits always occur in order, changes to the synchronised document are intended to be local-first.
This means you can change the document without an internet connection, and your changes will be sent and you will receive
external changes once you re-establish an internet connection. Since there's no strict ordering of which changes were made first,
we need to use CRDTs so that each device can eventually agree on state of the document.
