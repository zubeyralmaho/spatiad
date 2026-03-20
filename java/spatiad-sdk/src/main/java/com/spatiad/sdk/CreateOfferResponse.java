package com.spatiad.sdk;

import com.fasterxml.jackson.annotation.JsonProperty;

public class CreateOfferResponse {

    @JsonProperty("offer_id")
    private String offerId;

    public CreateOfferResponse() {
    }

    public String getOfferId() {
        return offerId;
    }

    public void setOfferId(String offerId) {
        this.offerId = offerId;
    }
}
