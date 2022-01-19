#![no_std]

extern crate alloc;

const NFT_AMOUNT: u32 = 1;
const ROYALTIES_MAX: u32 = 10_000;
const IPFS_GATEWAY_HOST: &[u8] = "https://ipfs.io/ipfs/".as_bytes();
const IPFS_SCHEME: &[u8] = "ipfs://".as_bytes();
const METADATA_KEY_NAME: &[u8] = "metadata:".as_bytes();
const METADATA_FILE_EXTENSION: &[u8] = ".json".as_bytes();
const ATTR_SEPARATOR: &[u8] = ";".as_bytes();
const URI_SLASH: &[u8] = "/".as_bytes();
const TAGS_KEY_NAME: &[u8] = "tags:".as_bytes();
const DEFAULT_IMG_FILE_EXTENSION: &[u8] = ".png".as_bytes();

elrond_wasm::imports!();
elrond_wasm::derive_imports!();

#[elrond_wasm::contract]
pub trait ElvenTools {
    #[init]
    fn init(
        &self,
        image_base_cid: ManagedBuffer,
        metadata_base_cid: ManagedBuffer,
        amount_of_tokens: u32,
        tokens_limit_per_address: u32,
        royalties: BigUint,
        selling_price: BigUint,
        #[var_args] file_extension: OptionalArg<ManagedBuffer>,
        #[var_args] tags: OptionalArg<ManagedBuffer>,
        #[var_args] provenance_hash: OptionalArg<ManagedBuffer>,
    ) -> SCResult<()> {
        require!(royalties <= ROYALTIES_MAX, "Royalties cannot exceed 100%!");
        require!(
            amount_of_tokens >= 1,
            "Amount of tokens to mint should be at least 1!"
        );
        require!(
            tokens_limit_per_address >= 1,
            "Tokens limit per address should be at least 1!"
        );

        self.image_base_cid().set(&image_base_cid);
        self.metadata_base_cid().set(&metadata_base_cid);
        self.amount_of_tokens().set(&amount_of_tokens);
        self.tokens_limit_per_address()
            .set(&tokens_limit_per_address);
        self.provenance_hash()
            .set(&provenance_hash.into_option().unwrap_or_default());
        self.royalties().set(&royalties);
        self.selling_price().set(&selling_price);
        self.tags().set(&tags.into_option().unwrap_or_default());
        self.file_extension().set(
            &file_extension
                .into_option()
                .unwrap_or_else(|| ManagedBuffer::new_from_bytes(DEFAULT_IMG_FILE_EXTENSION)),
        );
        let paused = true;
        self.paused().set(&paused);

        // TODO: enable when shuffle is ready - replace '1' with random index
        // self.shuffle();

        let first_index = 1;
        self.next_index_to_mint().set(&first_index);

        Ok(())
    }

    // Issue main collection token/handler
    #[only_owner]
    #[payable("EGLD")]
    #[endpoint(issueToken)]
    fn issue_token(
        &self,
        #[payment] issue_cost: BigUint,
        token_name: ManagedBuffer,
        token_ticker: ManagedBuffer,
    ) -> SCResult<AsyncCall> {
        require!(self.nft_token_id().is_empty(), "Token already issued!");

        Ok(self
            .send()
            .esdt_system_sc_proxy()
            .issue_non_fungible(
                issue_cost,
                &token_name,
                &token_ticker,
                NonFungibleTokenProperties {
                    can_freeze: false,
                    can_wipe: false,
                    can_pause: false,
                    can_change_owner: false,
                    can_upgrade: false,
                    can_add_special_roles: true,
                },
            )
            .async_call()
            .with_callback(self.callbacks().issue_callback()))
    }

    #[only_owner]
    #[endpoint(setLocalRoles)]
    fn set_local_roles(&self) -> SCResult<AsyncCall> {
        require!(!self.nft_token_id().is_empty(), "Token not issued!");

        Ok(self
            .send()
            .esdt_system_sc_proxy()
            .set_special_roles(
                &self.blockchain().get_sc_address(),
                &self.nft_token_id().get(),
                (&[EsdtLocalRole::NftCreate][..]).into_iter().cloned(),
            )
            .async_call())
    }

    #[only_owner]
    #[endpoint(pauseMinting)]
    fn pause_minting(&self) -> SCResult<()> {
        let paused = true;
        self.paused().set(&paused);

        Ok(())
    }

    #[only_owner]
    #[endpoint(startMinting)]
    fn start_minting(&self) -> SCResult<()> {
        self.paused().clear();

        Ok(())
    }

    #[only_owner]
    #[endpoint(setDrop)]
    fn set_drop(&self, amount_of_tokens_per_drop: u32) -> SCResult<()> {
        self.minted_indexes_by_drop().clear();
        self.amount_of_tokens_per_drop()
            .set(&amount_of_tokens_per_drop);

        Ok(())
    }

    #[only_owner]
    #[endpoint(unsetDrop)]
    fn unset_drop(&self) -> SCResult<()> {
        self.amount_of_tokens_per_drop().clear();
        self.minted_indexes_by_drop().clear();

        Ok(())
    }

    // The owner can change the price, for example, a new price for the next nft drop.
    #[only_owner]
    #[endpoint(setNewPrice)]
    fn set_new_price(&self, price: BigUint) -> SCResult<()> {
        self.selling_price().set(&price);

        Ok(())
    }

    // As an owner of the smart contract, you can send randomly minted NFTs to chosen addresses.
    #[only_owner]
    #[endpoint(giveaway)]
    fn giveaway(&self, address: ManagedAddress, amount_of_tokens: u32) -> SCResult<()> {
        require!(!self.nft_token_id().is_empty(), "Token not issued!");

        let token = self.nft_token_id().get();
        let roles = self.blockchain().get_esdt_local_roles(&token);

        require!(
            roles.has_role(&EsdtLocalRole::NftCreate),
            "NFTCreate role not set!"
        );

        require!(
            self.get_current_left_tokens_amount() >= amount_of_tokens,
            "All tokens have been minted already (totally or per drop)!"
        );

        for _ in 0..amount_of_tokens {
            self.mint_single_nft(BigUint::zero(), OptionalArg::Some(address.clone()))
                .unwrap();
        }

        Ok(())
    }

    // As an owner, claim Smart Contract balance.
    #[only_owner]
    #[endpoint(claimScFunds)]
    fn claim_sc_funds(&self) -> SCResult<()> {
        self.send().direct_egld(
            &self.blockchain().get_caller(),
            &self
                .blockchain()
                .get_sc_balance(&TokenIdentifier::egld(), 0),
            &[],
        );

        Ok(())
    }

    // Main mint function - takes the payment sum for all tokens to mint.
    #[payable("EGLD")]
    #[endpoint(mint)]
    fn mint(
        &self,
        #[payment_amount] payment_amount: BigUint,
        #[var_args] token_amount: OptionalArg<u32>,
    ) -> SCResult<()> {
        require!(
            self.paused().is_empty(),
            "The minting is paused or haven't started yet!"
        );
        require!(!self.nft_token_id().is_empty(), "Token not issued!");

        let token = self.nft_token_id().get();

        let roles = self.blockchain().get_esdt_local_roles(&token);

        require!(
            roles.has_role(&EsdtLocalRole::NftCreate),
            "ESDTNFTCreate role not set!"
        );

        let mut tokens = token_amount.into_option().unwrap_or_default();

        require!(
            self.get_current_left_tokens_amount() >= tokens,
            "All tokens have been minted already (totally or per drop)!"
        );

        let caller = self.blockchain().get_caller();

        let minted_per_address = self.minted_per_address(&caller).get();
        let tokens_limit_per_address = self.tokens_limit_per_address().get();

        let tokens_left_to_mint = tokens_limit_per_address - minted_per_address;

        if (tokens < 1) {
            tokens = 1
        }

        require!(
            tokens_left_to_mint >= tokens,
            "You can't mint such an amount of tokens. Check the limits by one address!"
        );

        let single_payment_amount = payment_amount / tokens;

        let price_tag = self.selling_price().get();
        require!(
            single_payment_amount == price_tag,
            "Invalid amount as payment"
        );

        for _ in 0..tokens {
            self.mint_single_nft(single_payment_amount.clone(), OptionalArg::None)
                .unwrap();
        }

        Ok(())
    }

    // Private single token mint function. It is also used for the giveaway.
    fn mint_single_nft(
        &self,
        payment_amount: BigUint,
        #[var_args] giveaway_address: OptionalArg<ManagedAddress>,
    ) -> SCResult<()> {
        let amount = &BigUint::from(NFT_AMOUNT);

        let token = self.nft_token_id().get();
        let token_name = self.nft_token_name().get();

        let royalties = self.royalties().get();

        let attributes = self.build_attributes_buffer();

        let attributes_hash = self
            .crypto()
            .sha256(&attributes);
        let hash_buffer = attributes_hash.as_managed_buffer();

        let uris = self.build_uris_vec();

        let nonce = self.send().esdt_nft_create(
            &token,
            &amount,
            &token_name,
            &royalties,
            &hash_buffer,
            &attributes,
            &uris,
        );

        // Choose next index to mint here (random)
        self.handle_next_index_setup();

        let giveaway_address = giveaway_address
            .into_option()
            .unwrap_or_else(|| ManagedAddress::zero());

        let nft_token_id = self.nft_token_id().get();
        let caller = self.blockchain().get_caller();

        let receiver;

        if (giveaway_address.is_zero()) {
            receiver = &caller;
        } else {
            receiver = &giveaway_address;
        }

        self.send().direct(
            &receiver,
            &nft_token_id,
            nonce,
            &BigUint::from(NFT_AMOUNT),
            &[],
        );

        self.minted_per_address(&caller).update(|sum| *sum += 1);

        if (payment_amount > 0) {
            let payment_nonce: u64 = 0;
            let payment_token = &TokenIdentifier::egld();

            let owner = self.blockchain().get_owner_address();
            self.send()
                .direct(&owner, &payment_token, payment_nonce, &payment_amount, &[]);
        }

        Ok(())
    }

    #[endpoint(shuffle)]
    fn shuffle(&self) -> SCResult<()> {
        // TODO: enable when RandomnessSource is available
        // Set next item to mint by random from whole tokens amount left

        Ok(())
    }

    #[callback]
    fn issue_callback(&self, #[call_result] result: ManagedAsyncCallResult<TokenIdentifier>) {
        match result {
            ManagedAsyncCallResult::Ok(token_id) => {
                self.nft_token_id().set(&token_id);
            }
            ManagedAsyncCallResult::Err(_) => {
                let caller = self.blockchain().get_owner_address();
                let (returned_tokens, token_id) = self.call_value().payment_token_pair();
                if token_id.is_egld() && returned_tokens > 0 {
                    self.send()
                        .direct(&caller, &token_id, 0, &returned_tokens, &[]);
                }
            }
        }
    }

    fn handle_next_index_setup(&self) {
        // TODO: randomize, remove used indexes from randomization - should be done in the shuffle function
        let minted_index = self.next_index_to_mint().get();
        let drop_amount = self.amount_of_tokens_per_drop().get();
        self.minted_indexes().insert(minted_index);
        if (drop_amount > 0) {
            self.minted_indexes_by_drop().insert(minted_index);
        }

        let next_index = minted_index + 1;
        self.next_index_to_mint().set(&next_index);
    }

    fn build_uris_vec(&self) -> ManagedVec<ManagedBuffer> {
        use alloc::string::ToString;

        let index_to_mint = self.next_index_to_mint().get();
        let mut uris = ManagedVec::new();

        let cid = self.image_base_cid().get();
        let uri_slash = ManagedBuffer::new_from_bytes(URI_SLASH);
        let image_file_extension = self.file_extension().get();
        let file_index = ManagedBuffer::from(index_to_mint.to_string().as_bytes());

        let mut img_ipfs_gateway_uri = ManagedBuffer::new_from_bytes(IPFS_GATEWAY_HOST);
        img_ipfs_gateway_uri.append(&cid);
        img_ipfs_gateway_uri.append(&uri_slash);
        img_ipfs_gateway_uri.append(&file_index);
        img_ipfs_gateway_uri.append(&image_file_extension);

        let mut img_ipfs_uri = ManagedBuffer::new_from_bytes(IPFS_SCHEME);
        img_ipfs_uri.append(&cid);
        img_ipfs_uri.append(&uri_slash);
        img_ipfs_uri.append(&file_index);
        img_ipfs_uri.append(&image_file_extension);

        uris.push(img_ipfs_gateway_uri);
        uris.push(img_ipfs_uri);

        uris
    }

    // TODO: this can be probably optimized with attributes struct, had problems with decoding on the api side
    fn build_attributes_buffer(&self) -> ManagedBuffer {
        use alloc::string::ToString;

        let index_to_mint = self.next_index_to_mint().get();
        let metadata_key_name = ManagedBuffer::new_from_bytes(METADATA_KEY_NAME);
        let metadata_index_file =
            ManagedBuffer::new_from_bytes(index_to_mint.to_string().as_bytes());
        let metadata_file_extension = ManagedBuffer::new_from_bytes(METADATA_FILE_EXTENSION);
        let metadata_cid = self.metadata_base_cid().get();
        let separator = ManagedBuffer::new_from_bytes(ATTR_SEPARATOR);
        let metadata_slash = ManagedBuffer::new_from_bytes(URI_SLASH);
        let tags_key_name = ManagedBuffer::new_from_bytes(TAGS_KEY_NAME);

        let mut attributes = ManagedBuffer::new();
        attributes.append(&tags_key_name);
        attributes.append(&self.tags().get());
        attributes.append(&separator);
        attributes.append(&metadata_key_name);
        attributes.append(&metadata_cid);
        attributes.append(&metadata_slash);
        attributes.append(&metadata_index_file);
        attributes.append(&metadata_file_extension);

        attributes
    }

    fn get_current_left_tokens_amount(&self) -> u32 {
        let drop_amount = self.amount_of_tokens_per_drop().get();
        let tokens_left;
        let paused = true;
        if (drop_amount > 0) {
            tokens_left = self.drop_tokens_left().ok().unwrap_or_default();
        } else {
            tokens_left = self.total_tokens_left().ok().unwrap_or_default();
        }

        if (tokens_left <= 0) {
            self.paused().set(&paused);
        }

        tokens_left
    }

    #[view(dropTokensLeft)]
    fn drop_tokens_left(&self) -> SCResult<u32> {
        let minted_tokens = self.minted_indexes_by_drop().len();
        let amount_of_tokens = self.amount_of_tokens_per_drop().get();
        let left_tokens: u32 = amount_of_tokens - minted_tokens as u32;

        Ok(left_tokens)
    }

    #[view(totalTokensLeft)]
    fn total_tokens_left(&self) -> SCResult<u32> {
        let minted_tokens = self.minted_indexes().len();
        let amount_of_tokens = self.amount_of_tokens().get();
        let left_tokens: u32 = amount_of_tokens - minted_tokens as u32;

        Ok(left_tokens)
    }

    #[view(getNftTokenId)]
    #[storage_mapper("nftTokenId")]
    fn nft_token_id(&self) -> SingleValueMapper<TokenIdentifier>;

    #[view(getNftTokenName)]
    #[storage_mapper("nftTokenName")]
    fn nft_token_name(&self) -> SingleValueMapper<ManagedBuffer>;

    #[view(getNftPrice)]
    #[storage_mapper("nftPrice")]
    fn selling_price(&self) -> SingleValueMapper<BigUint>;

    #[view(provenanceHash)]
    #[storage_mapper("provenanceHash")]
    fn provenance_hash(&self) -> SingleValueMapper<ManagedBuffer>;

    #[storage_mapper("iamgeBaseCid")]
    fn image_base_cid(&self) -> SingleValueMapper<ManagedBuffer>;

    #[storage_mapper("metadaBaseCid")]
    fn metadata_base_cid(&self) -> SingleValueMapper<ManagedBuffer>;

    #[storage_mapper("file_extension")]
    fn file_extension(&self) -> SingleValueMapper<ManagedBuffer>;

    #[storage_mapper("amountOfTokens")]
    fn amount_of_tokens(&self) -> SingleValueMapper<u32>;

    #[storage_mapper("mintedIndexes")]
    fn minted_indexes(&self) -> SetMapper<u32>;

    #[storage_mapper("mintedIndexesByDrop")]
    fn minted_indexes_by_drop(&self) -> SetMapper<u32>;

    #[storage_mapper("nextIndexToMint")]
    fn next_index_to_mint(&self) -> SingleValueMapper<u32>;

    #[storage_mapper("royalties")]
    fn royalties(&self) -> SingleValueMapper<BigUint>;

    #[storage_mapper("paused")]
    fn paused(&self) -> SingleValueMapper<bool>;

    #[storage_mapper("tags")]
    fn tags(&self) -> SingleValueMapper<ManagedBuffer>;

    #[storage_mapper("tokensLimitPerAddress")]
    fn tokens_limit_per_address(&self) -> SingleValueMapper<u32>;

    #[storage_mapper("mintedPerAddress")]
    fn minted_per_address(&self, address: &ManagedAddress) -> SingleValueMapper<u32>;

    #[storage_mapper("amountOfTokensPerDrop")]
    fn amount_of_tokens_per_drop(&self) -> SingleValueMapper<u32>;
}
